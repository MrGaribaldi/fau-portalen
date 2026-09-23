//! Connection building shared by every persistence entry point.
//!
//! Isolated here so that "an error must never repeat the DSN or a password" is
//! enforced in one place instead of reimplemented per caller -- the migration
//! runner today, a runtime pool later. `sqlx::Error`'s own `Display` for a connect
//! failure is not safe to surface directly: depending on the underlying driver
//! error it can echo back the connection string it was given, credentials
//! included. This module never does that: a caller gets a fixed classification, not
//! the underlying message.

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

/// PostgreSQL SQLSTATEs for a rejected login: `invalid_password` and
/// `invalid_authorization_specification`. Fixed identifiers from PostgreSQL's own
/// error-codes table -- never a value the server or the caller supplied, so safe to
/// match on and to surface. `pub(crate)` so `contract.rs`'s schema-contract-check
/// classification can reuse the same two constants rather than repeating the
/// literal SQLSTATEs a second time.
pub(crate) const SQLSTATE_INVALID_PASSWORD: &str = "28P01";
pub(crate) const SQLSTATE_INVALID_AUTHORIZATION: &str = "28000";

/// Why building or using a connection failed. Deliberately coarse: enough to tell
/// an operator what kind of problem this is, never enough to repeat a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectErrorKind {
    /// The URL itself could not be parsed as a PostgreSQL connection string.
    InvalidUrl,
    /// A network/IO failure -- refused, reset, unreachable, DNS, and so on.
    Io,
    /// TLS negotiation failed.
    Tls,
    /// The server rejected the credentials (SQLSTATE `28P01` or `28000`).
    Authentication,
    /// The attempt did not complete within the caller's own timeout, distinct from
    /// any PostgreSQL-side lock or statement timeout.
    Timeout,
    /// Any other connection failure not classified above -- e.g. the database does
    /// not exist, or some other server-side rejection unrelated to credentials.
    Other,
}

impl std::fmt::Display for ConnectErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::InvalidUrl => "invalid url",
            Self::Io => "network error",
            Self::Tls => "tls error",
            Self::Authentication => "authentication failed",
            Self::Timeout => "connection timed out",
            Self::Other => "connection failed",
        };
        f.write_str(text)
    }
}

/// Classifies a connection failure without ever retaining or repeating its
/// message -- which, depending on the underlying driver error, can echo back the
/// connection string it was given, credentials included. The SQLSTATE code itself
/// (when there is one) is safe to inspect: it is a fixed enumeration from
/// PostgreSQL, never a value supplied by a caller.
pub fn classify_connect_error(e: &sqlx::Error) -> ConnectErrorKind {
    match e {
        sqlx::Error::Io(_) => ConnectErrorKind::Io,
        sqlx::Error::Tls(_) => ConnectErrorKind::Tls,
        sqlx::Error::Database(db_err) => match db_err.code().as_deref() {
            Some(SQLSTATE_INVALID_PASSWORD) | Some(SQLSTATE_INVALID_AUTHORIZATION) => {
                ConnectErrorKind::Authentication
            }
            _ => ConnectErrorKind::Other,
        },
        _ => ConnectErrorKind::Other,
    }
}

/// Parses `url` and layers `options` (e.g. `lock_timeout`) on top, without ever
/// retaining or displaying the credentials `url` carries.
pub fn connect_options(
    url: &str,
    options: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<PgConnectOptions, ConnectErrorKind> {
    let opts: PgConnectOptions = url.parse().map_err(|_| ConnectErrorKind::InvalidUrl)?;
    Ok(opts.options(options))
}

/// Builds `serve`'s runtime pool *lazily*: `connect_lazy_with` never dials the
/// database, it only validates and stores the connect options, so a database that
/// is merely unreachable at startup is not a startup failure (design section 4).
/// The first real connection attempt happens on first use -- Task 7's
/// schema-contract startup check (`read_contract_version`) is that first caller;
/// Task 9's readiness check reuses the same pool afterwards.
pub fn lazy_pool(url: &str, max_connections: u32) -> Result<PgPool, ConnectErrorKind> {
    let options = connect_options(url, [])?;
    Ok(PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_lazy_with(options))
}

/// A diagnosable but safe description of a `sqlx::Error`: the SQLSTATE when the
/// database gave one, otherwise a fixed word for the kind of failure. Never the
/// error's own `Display`, which can quote the failing statement, a bound value, or
/// -- for some connection failures -- the connection string itself. Shared by the
/// migration runner's error messages (`migrate.rs`) and the schema-contract startup
/// gate's (`contract.rs`, `fau-app`'s `main.rs`), so there is exactly one place that
/// decides what is safe to print about a `sqlx::Error`.
pub fn safe_error_kind(e: &sqlx::Error) -> String {
    match e {
        sqlx::Error::Database(db_err) => match db_err.code() {
            Some(code) => format!("sqlstate {code}"),
            None => "database error".to_owned(),
        },
        sqlx::Error::Io(_) => "io error".to_owned(),
        sqlx::Error::Tls(_) => "tls error".to_owned(),
        sqlx::Error::PoolTimedOut => "pool timed out".to_owned(),
        sqlx::Error::PoolClosed => "pool closed".to_owned(),
        sqlx::Error::ColumnDecode { .. } => "column decode error".to_owned(),
        sqlx::Error::Decode(_) => "decode error".to_owned(),
        _ => "query failed".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    /// A minimal `DatabaseError` double so `classify_connect_error`'s SQLSTATE
    /// branch can be exercised without a live server.
    #[derive(Debug)]
    struct FakeDbError {
        code: &'static str,
    }

    impl std::fmt::Display for FakeDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "fake database error {}", self.code)
        }
    }

    impl std::error::Error for FakeDbError {}

    impl sqlx::error::DatabaseError for FakeDbError {
        fn message(&self) -> &str {
            "fake"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.code))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn database_error(code: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(FakeDbError { code }))
    }

    #[test]
    fn invalid_url_is_reported_without_echoing_it() {
        let err = connect_options("not a postgres url", []).unwrap_err();
        assert_eq!(err, ConnectErrorKind::InvalidUrl);
        assert!(!format!("{err}").contains("not a postgres url"));
    }

    #[test]
    fn valid_url_carries_the_requested_options() {
        let opts = connect_options(
            "postgres://u:hunter2@127.0.0.1:5432/db",
            [("lock_timeout", "10000".to_owned())],
        )
        .expect("valid url");
        // `PgConnectOptions` has no public accessor for the options string, so this
        // reaches into its `Debug` output -- a test-only use, never something this
        // crate does at runtime (that `Debug` also carries the password).
        assert!(format!("{opts:?}").contains("lock_timeout=10000"));
    }

    #[test]
    fn io_error_classifies_as_io() {
        let err = sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ));
        assert_eq!(classify_connect_error(&err), ConnectErrorKind::Io);
    }

    #[test]
    fn invalid_password_sqlstate_classifies_as_authentication() {
        assert_eq!(
            classify_connect_error(&database_error(SQLSTATE_INVALID_PASSWORD)),
            ConnectErrorKind::Authentication
        );
    }

    #[test]
    fn invalid_authorization_sqlstate_classifies_as_authentication() {
        assert_eq!(
            classify_connect_error(&database_error(SQLSTATE_INVALID_AUTHORIZATION)),
            ConnectErrorKind::Authentication
        );
    }

    #[test]
    fn an_unrelated_sqlstate_classifies_as_other() {
        assert_eq!(
            classify_connect_error(&database_error("42601")),
            ConnectErrorKind::Other
        );
    }

    #[test]
    fn safe_error_kind_names_the_sqlstate_when_the_database_gave_one() {
        assert_eq!(safe_error_kind(&database_error("42501")), "sqlstate 42501");
    }

    #[test]
    fn safe_error_kind_is_a_fixed_word_for_io() {
        let err = sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ));
        assert_eq!(safe_error_kind(&err), "io error");
    }

    #[test]
    fn safe_error_kind_is_a_fixed_word_for_pool_timed_out() {
        assert_eq!(
            safe_error_kind(&sqlx::Error::PoolTimedOut),
            "pool timed out"
        );
    }

    #[test]
    fn safe_error_kind_is_a_fixed_word_for_decode_and_never_the_display() {
        let err = sqlx::Error::Decode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "sentinel-decode-detail",
        )));
        let kind = safe_error_kind(&err);
        assert_eq!(kind, "decode error");
        assert!(!kind.contains("sentinel-decode-detail"));
    }
}
