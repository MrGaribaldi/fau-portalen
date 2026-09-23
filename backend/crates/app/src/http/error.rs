//! The JSON error contract (design section 11): `{code, params, request_id}` and
//! nothing else -- never display text, because the client renders the sentence.

use std::collections::BTreeMap;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use fau_domain::error_code::{ErrorCode, ParamValue};
use serde::Serialize;

use super::request_context::current_request_id;

/// An API error: a stable code, bounded parameters and an HTTP status. Fields are
/// private on purpose -- `params` is typed as a [`ParamValue`] map, so a value can
/// never be anything but one of that enum's bounded variants, and every error
/// carries the request id via [`current_request_id`], not a field a constructor
/// could forget to set.
pub struct ApiError {
    code: ErrorCode,
    params: BTreeMap<String, ParamValue>,
    status: StatusCode,
}

impl ApiError {
    pub fn new(code: ErrorCode, status: StatusCode) -> Self {
        Self {
            code,
            params: BTreeMap::new(),
            status,
        }
    }

    pub fn not_found() -> Self {
        Self::new(ErrorCode::NotFound, StatusCode::NOT_FOUND)
    }

    pub fn internal_error() -> Self {
        Self::new(ErrorCode::InternalError, StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// The wire shape: `{code, params, request_id}`. A separate type from `ApiError`
/// itself so `ApiError`'s fields can stay private while this only ever borrows what
/// it needs to serialise.
#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a ErrorCode,
    params: &'a BTreeMap<String, ParamValue>,
    request_id: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            code: &self.code,
            params: &self.params,
            request_id: current_request_id(),
        };
        (self.status, Json(body)).into_response()
    }
}

/// The shared fallback for `/api/v1/*`, `/assets/*` and any other unmatched
/// top-level path: JSON 404, never HTML (spec section 8).
pub async fn json_not_found() -> ApiError {
    ApiError::not_found()
}

/// `tower_http::catch_panic::CatchPanicLayer`'s custom panic handler
/// (`super::router`): turns a caught handler panic into the same JSON error contract
/// as any other API error, never the default catch-panic behaviour's plain-text
/// body. Logs one `ERROR` event -- carrying `request_id` automatically, via the
/// ambient per-request span `request_context::middleware` enters (see
/// `crate::telemetry`) -- but deliberately never the panic payload itself: a handler
/// can panic while holding arbitrary data (a document body, a bound SQL parameter,
/// an internal error's `Display`, ...), so nothing about *why* it panicked is safe to
/// log or return without knowing what it was.
pub fn handle_panic(_payload: Box<dyn std::any::Any + Send + 'static>) -> Response {
    tracing::error!("a request handler panicked");
    ApiError::internal_error().into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Narrow, function-level check: calls `handle_panic` directly with a fake
    /// payload, outside any router, `CatchPanicLayer` or the process panic hook. It
    /// only proves this one function's own mapping from a caught payload to a
    /// response -- it is not, and cannot be, proof that a *real* panic is ever
    /// caught in the first place or that the default panic hook's plain-text stderr
    /// output is suppressed. That end-to-end proof, via a real `/test/panic` route,
    /// is `tests/panic_handling.rs` (`--features test-routes`).
    #[tokio::test]
    async fn handle_panic_returns_the_json_error_contract_without_the_panic_payload() {
        let response = handle_panic(Box::new("sentinel panic payload".to_owned()));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the panic response body");
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("a JSON body, not the raw panic text");

        assert_eq!(body["code"], "internal_error");
        assert!(body["request_id"].is_string());
        let raw = body.to_string();
        assert!(
            !raw.contains("sentinel panic payload"),
            "the panic payload leaked into the response: {raw}"
        );
    }
}
