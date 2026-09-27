//! Wrapped data keys (docs/key-service-design.md §3.2). Callers pass their own
//! connection, so a unit's key is stored in the same transaction as the first row it
//! protects.

use fau_crypto::{Unit, WrappedKey};
use sqlx::PgConnection;

#[derive(Debug, thiserror::Error)]
pub enum WrappedKeyError {
    #[error("this unit has no data key")]
    NoDataKey,
    #[error("a stored wrapped key is not transit ciphertext")]
    Corrupt,
    #[error("database error")]
    Db(#[from] sqlx::Error),
}

fn row(unit: &Unit) -> Result<(&'static str, Option<String>), WrappedKeyError> {
    unit.storage().ok_or(WrappedKeyError::NoDataKey)
}

fn decode(bytes: Vec<u8>) -> Result<WrappedKey, WrappedKeyError> {
    WrappedKey::new(String::from_utf8(bytes).map_err(|_| WrappedKeyError::Corrupt)?)
        .ok_or(WrappedKeyError::Corrupt)
}

pub async fn load_wrapped_key(
    conn: &mut PgConnection,
    unit: &Unit,
) -> Result<Option<WrappedKey>, WrappedKeyError> {
    let (u, scope) = row(unit)?;
    let found: Option<Vec<u8>> = sqlx::query_scalar(
        "select wrapped_key from wrapped_keys where tenant_id = $1 and unit = $2 and coalesce(scope, '') = coalesce($3, '')")
        .bind(unit.tenant()).bind(u).bind(scope).fetch_optional(&mut *conn).await?;
    found.map(decode).transpose()
}

pub async fn store_wrapped_key(
    conn: &mut PgConnection,
    unit: &Unit,
    key: &WrappedKey,
) -> Result<WrappedKey, WrappedKeyError> {
    let (u, scope) = row(unit)?;
    sqlx::query(
        "insert into wrapped_keys (tenant_id, unit, scope, wrapped_key) values ($1, $2, $3, $4)
         on conflict (tenant_id, unit, coalesce(scope, '')) do nothing",
    )
    .bind(unit.tenant())
    .bind(u)
    .bind(&scope)
    .bind(key.as_str().as_bytes())
    .execute(&mut *conn)
    .await?;
    load_wrapped_key(conn, unit)
        .await?
        .ok_or(WrappedKeyError::Corrupt)
}
