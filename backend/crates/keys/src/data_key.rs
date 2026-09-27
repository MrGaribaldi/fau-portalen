//! The data-key flow (docs/key-service-design.md §3.2): load the unit's wrapped key, or
//! create one; return the plaintext through the session cache.

use fau_crypto::{DataKey, Unit};
use fau_persistence::keys::{load_wrapped_key, store_wrapped_key, WrappedKeyError};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::cache::KeyCache;
use crate::client::{KeyError, Keys};

#[derive(Debug, thiserror::Error)]
pub enum DataKeyError {
    #[error(transparent)]
    Keys(#[from] KeyError),
    #[error(transparent)]
    Store(#[from] WrappedKeyError),
}

pub async fn data_key(
    conn: &mut PgConnection,
    keys: &Keys,
    cache: &KeyCache,
    session: Uuid,
    unit: &Unit,
) -> Result<DataKey, DataKeyError> {
    if let Some(wrapped) = load_wrapped_key(conn, unit).await? {
        return Ok(cache.get(session, unit, &wrapped).await?);
    }
    let (fresh, wrapped) = keys.new_data_key(unit).await?;
    let stored = store_wrapped_key(conn, unit, &wrapped).await?;
    if stored == wrapped {
        cache.put(session, *unit, fresh.clone());
        Ok(fresh)
    } else {
        // Another session stored its key first: ours is discarded and never used.
        Ok(cache.get(session, unit, &stored).await?)
    }
}
