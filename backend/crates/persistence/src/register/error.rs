//! The register's one error type. Its `Display` is fixed text per variant: never a bound
//! value, a payload, a name or an address.

use crate::pool::safe_error_kind;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    /// `pool::safe_error_kind`'s fixed description: a SQLSTATE or a fixed word.
    #[error("database error ({0})")]
    Database(String),
    /// A value read back from the register did not decode: a bug or a schema drift.
    #[error("a register row did not decode")]
    Decode,
    /// The plan names a row the register does not hold, or a `Ref::New` no op creates.
    #[error("the plan names a row the register does not hold")]
    UnknownRow,
    /// An NSR payload was not UTF-8 JSON, so it cannot be staged as `jsonb`.
    #[error("an NSR payload is not valid UTF-8")]
    PayloadNotUtf8,
}

impl RegisterError {
    /// The session's `lock_timeout` (SQLSTATE 55P03) or `statement_timeout` (57014) ended a
    /// statement: the run was blocked, not refused.
    pub fn is_timeout(&self) -> bool {
        matches!(self, RegisterError::Database(kind)
            if kind == "sqlstate 55P03" || kind == "sqlstate 57014")
    }
}

impl From<sqlx::Error> for RegisterError {
    fn from(e: sqlx::Error) -> Self {
        RegisterError::Database(safe_error_kind(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_database_error_carries_only_the_sqlstate() {
        let e: RegisterError = crate::pool::test_support::database_error("42501").into();
        assert_eq!(e, RegisterError::Database("sqlstate 42501".to_owned()));
        assert_eq!(e.to_string(), "database error (sqlstate 42501)");
    }
}
