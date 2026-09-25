//! The one way a register command opens its database connection: a dedicated `PgConnection`
//! (it holds the sync's session advisory lock), with its own timeouts so a blocked run fails
//! with a fixed code instead of hanging the CronJob until its deadline (final review, item 3).

use std::str::FromStr;

use sqlx::postgres::PgConnectOptions;
use sqlx::{ConnectOptions, PgConnection};

use super::error::RegisterError;

/// No register statement runs anywhere near this long; one that does is stuck.
pub const STATEMENT_TIMEOUT: &str = "60s";
/// How long a statement waits for a row or table lock another session holds.
pub const LOCK_TIMEOUT: &str = "10s";

/// Connects to `url` with [`STATEMENT_TIMEOUT`] and [`LOCK_TIMEOUT`] set for the session, as
/// startup options, so they apply before the first statement. The error never carries the
/// URL: `RegisterError`'s conversion keeps only a fixed word.
pub async fn connect(url: &str) -> Result<PgConnection, RegisterError> {
    let options = PgConnectOptions::from_str(url)?.options([
        ("statement_timeout", STATEMENT_TIMEOUT),
        ("lock_timeout", LOCK_TIMEOUT),
    ]);
    Ok(options.connect().await?)
}
