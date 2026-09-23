//! The HTTP surface: liveness, the static Bokmal placeholder, the JSON error
//! contract and the routing rules (design sections 3, 4, 8, 9 and 11).
//!
//! Routing order matters, and is deliberate (spec section 8 / ADR-001): `/api/v1/`
//! and `/assets/` each carry an explicit JSON-404 fallback of their own, registered
//! *before* the `/app` routes, so a missing asset or an unknown API route can never
//! resolve to HTML. The top-level fallback is JSON too, on the same reasoning -- an
//! unknown top-level path is more likely a mistyped API call than a browser
//! navigation, and ADR-001 forbids an HTML fallback outside `/app/`.
//!
//! `/app` is deliberately **not** built with `.nest("/app", Router::new().fallback(..))`
//! the way `/api/v1` and `/assets` are: axum's nest matching has a well-known
//! trailing-slash gap where a request for exactly the nest prefix plus a slash
//! (`/app/`, nothing after) falls through to the *outer* router's fallback instead
//! of the nested one, even though both `/app` (no slash) and `/app/x/y` (something
//! after) match correctly. ADR-001 names `/app/` as the entry path, so that one
//! path is exactly the one this gap would break.
//!
//! Three explicit routes on the outer router sidestep it: `/app` (bare), `/app/`
//! (confirmed empirically -- axum's `{*rest}` wildcard segment does not match a
//! *zero-length* remainder, so `/app/{*rest}` alone still 404s on `/app/` itself)
//! and `/app/{*rest}` for anything deeper.

pub mod error;
pub mod health;
pub mod placeholder;

use axum::routing::get;
use axum::Router;
use sqlx::PgPool;

use crate::readiness::ReadinessState;

/// Shared state for every handler. `Clone` because axum's `State` extractor
/// requires it; cheap to clone -- `ReadinessState` is an `Arc` internally and
/// `PgPool` is a handle around its own connection pool, not the pool itself.
#[derive(Clone)]
pub struct AppState {
    pub readiness: ReadinessState,
    pub pool: PgPool,
    pub version: &'static str,
}

/// Builds the full router. See the module doc comment for why the registration
/// order below is load-bearing, not incidental.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready))
        .route("/", get(placeholder::page))
        .nest("/api/v1", Router::new().fallback(error::json_not_found))
        .nest("/assets", Router::new().fallback(error::json_not_found))
        .route("/app", get(placeholder::page))
        .route("/app/", get(placeholder::page))
        .route("/app/{*rest}", get(placeholder::page))
        .fallback(error::json_not_found)
        .with_state(state)
}
