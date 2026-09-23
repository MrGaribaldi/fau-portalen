//! Request-scoped identity and the access log (design sections 10 and 11).
//!
//! A fresh UUIDv7 is minted for every request, attached to it as an extension,
//! echoed on the response's `x-request-id` header, and reused by every error body
//! ([`current_request_id`], read by `http::error::ApiError`) and by the access-log
//! line this module writes. An inbound `x-request-id` header is deliberately never
//! trusted: accepting one would let a caller inject an arbitrary or misleading value
//! into our own logs and error bodies, so [`middleware`] always mints its own id
//! rather than reading one from the request.
//!
//! [`middleware`] is registered as [`super::router`]'s outermost `.layer(...)`, added
//! after every route, nest and fallback -- axum wires a `Router::layer` to wrap each
//! already-selected endpoint's service (matched route *or* fallback), not the
//! router's own dispatch as a single unit, so [`axum::extract::MatchedPath`] is
//! already present in the request's extensions by the time this middleware runs for
//! a matched route, and correctly absent for a fallback (the `"<unmatched>"` case
//! below). It also sits outside `tower_http`'s `CatchPanicLayer` (see
//! `super::router`), so a caught handler panic is still logged and answered from
//! inside this middleware's request context.
//!
//! [`middleware`] enters a `tracing` span carrying `request_id` around the whole
//! request (`.instrument`, not a bare `.enter()` -- a span guard held across an
//! `.await` point is unsound on a multi-threaded runtime, since the executor can
//! move or interleave the task; `Instrument` re-enters the span around every poll
//! instead). `telemetry::JsonLineLayer` merges that span's fields into every log
//! line emitted while it is active -- including ones with no idea a request
//! triggered them, such as `readiness`'s probe warning or a caught panic -- which is
//! what lets `request_id` reach those lines without their call sites needing to
//! know it.

use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use uuid::Uuid;

use crate::telemetry::REQUEST_SPAN_TARGET;

/// The response header the request id is echoed on.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// One request's id, minted fresh by [`middleware`] -- never derived from any header
/// the caller sent. Attached to the request as an extension and, via the task-local
/// [`CURRENT_REQUEST_ID`], readable synchronously by [`current_request_id`].
#[derive(Debug, Clone, Copy)]
pub struct RequestId(pub Uuid);

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

tokio::task_local! {
    /// Scoped to one request by [`middleware`]. `http::error::ApiError` has no
    /// request extractor of its own -- it is built and turned into a response from
    /// deep inside ordinary application code -- so a task-local, not the request
    /// extension, is what lets [`current_request_id`] hand it the same id the
    /// response header and the access log line carry.
    static CURRENT_REQUEST_ID: RequestId;
}

/// The id of the request currently being handled. Falls back to a freshly minted id
/// outside of any request -- e.g. a unit test constructing an `ApiError` directly
/// without going through [`middleware`] -- which real traffic never hits, since
/// [`super::router`] wraps every route and fallback with this module's middleware.
pub fn current_request_id() -> String {
    CURRENT_REQUEST_ID
        .try_with(|id| id.to_string())
        .unwrap_or_else(|_| Uuid::now_v7().to_string())
}

/// Mints the request id, runs the request, echoes the id on the response header, and
/// writes the one access-log line design section 10 calls for: `timestamp`, `level`,
/// `service_version`, `request_id`, `route` (the matched-route template, or
/// `"<unmatched>"` for a fallback -- never the request's URI), `method`, `status` and
/// `duration_ms`. Deliberately absent: the URI, any query string, headers and
/// cookies -- see the module doc comment on why an inbound `x-request-id` is ignored
/// too. `service_version` is not passed explicitly here -- `telemetry::JsonLineLayer`
/// inserts it into every line unconditionally, this one included -- but `request_id`
/// still is, since this line is logged *after* the request-scoped span below has
/// already ended, so it is not itself covered by that span's merge.
pub async fn middleware(mut req: Request, next: Next) -> Response {
    let request_id = RequestId(Uuid::now_v7());
    req.extensions_mut().insert(request_id);

    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned())
        .unwrap_or_else(|| "<unmatched>".to_owned());
    let method = req.method().clone();

    // The request-scoped span: everything logged while `next.run(req)` is being
    // polled -- the handler itself, and anything it calls into, such as
    // `readiness`'s database check or a caught panic -- picks up `request_id`
    // through `telemetry::JsonLineLayer`, with nothing at those call sites needing
    // to pass it explicitly. `target: REQUEST_SPAN_TARGET` keeps the span itself
    // always created regardless of `LOG_LEVEL` -- see that constant's doc comment.
    let span =
        tracing::info_span!(target: REQUEST_SPAN_TARGET, "request", request_id = %request_id);

    let started = Instant::now();
    let mut response = CURRENT_REQUEST_ID
        .scope(request_id, next.run(req).instrument(span))
        .await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16() as u64;

    if let Ok(value) = HeaderValue::from_str(&request_id.to_string()) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }

    tracing::info!(
        request_id = %request_id,
        route = %route,
        method = %method,
        status,
        duration_ms,
        "request"
    );

    response
}
