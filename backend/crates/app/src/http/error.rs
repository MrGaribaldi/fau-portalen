//! The JSON error contract (design section 11): `{code, params, request_id}` and
//! nothing else -- never display text, because the client renders the sentence.

use std::collections::BTreeMap;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use fau_domain::error_code::{ErrorCode, ParamValue};
use serde::Serialize;
use uuid::Uuid;

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

/// The single source of an error response's `request_id`. A UUIDv7 for now, so the
/// field is always present; Task 10's request-id middleware replaces this with the
/// id already attached to the request by the time an error is turned into a
/// response, without `ApiError` itself changing.
fn current_request_id() -> String {
    Uuid::now_v7().to_string()
}

/// The shared fallback for `/api/v1/*`, `/assets/*` and any other unmatched
/// top-level path: JSON 404, never HTML (spec section 8).
pub async fn json_not_found() -> ApiError {
    ApiError::not_found()
}
