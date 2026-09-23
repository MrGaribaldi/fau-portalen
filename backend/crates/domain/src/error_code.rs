//! The API error contract's vocabulary, from the design's section 11 and #3439.
//!
//! An API error carries a stable, machine-readable code, bounded parameters and the
//! request ID -- never display text, because the client renders the sentence and an
//! endpoint that returns Norwegian prose silently breaks localisation. This module
//! declares what an error *is*; `fau-app`'s `http::error` module is what turns one
//! into an HTTP response. Kept in `domain` (which declares neither `axum` nor
//! `sqlx`) so the vocabulary of error codes is available to any layer without
//! pulling in HTTP.

use serde::Serialize;

/// A stable, machine-readable error code. `#[non_exhaustive]` so adding a variant
/// later is not a breaking change for a `match` elsewhere in the workspace.
/// `snake_case` on the wire needs no translation layer, and never changes once
/// shipped -- it is what the client's localisation table keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    /// The requested route, or resource, does not exist.
    NotFound,
    /// An unhandled server-side failure, including a caught handler panic. Carries
    /// no detail: whatever caused it is not known to be safe to describe to a
    /// client, so the message is a fixed word and the diagnosis lives in the
    /// server-side log line the same request id ties it to, never in the response.
    InternalError,
}

/// A bounded parameter value for an error response. An error can carry context (an
/// id, a count, a flag) but never an arbitrary payload -- restricting it to this
/// enum is what keeps that true structurally rather than by convention.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ParamValue {
    Int(i64),
    Str(String),
    Bool(bool),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_serialises_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::NotFound).unwrap(),
            "\"not_found\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::InternalError).unwrap(),
            "\"internal_error\""
        );
    }

    #[test]
    fn param_value_serialises_as_a_bare_primitive() {
        assert_eq!(serde_json::to_string(&ParamValue::Int(3)).unwrap(), "3");
        assert_eq!(
            serde_json::to_string(&ParamValue::Str("x".to_owned())).unwrap(),
            "\"x\""
        );
        assert_eq!(
            serde_json::to_string(&ParamValue::Bool(true)).unwrap(),
            "true"
        );
    }
}
