//! The static placeholder at `/`, also served under `/app/*` as the client-route
//! fallback shell (reserved for #3422; design section 8).

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

/// Bokmal, `<html lang="nb-NO">`, no JavaScript, no external references and no
/// configuration values -- embedded at compile time, never templated.
const PLACEHOLDER_HTML: &str = include_str!("placeholder.html");

/// Serves the placeholder with `cache-control: no-cache`: unlike hash-named assets,
/// HTML must always revalidate (ADR-001).
pub async fn page() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        PLACEHOLDER_HTML,
    )
        .into_response()
}
