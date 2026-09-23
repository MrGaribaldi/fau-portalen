//! Routes that exist only so an integration test can exercise behaviour that is
//! hard to trigger through the ordinary HTTP surface -- today, a deliberate handler
//! panic, so `tests/panic_handling.rs` can prove the whole panic-catching path (a
//! real `CatchPanicLayer`, the process-wide panic hook installed by
//! `telemetry::init`, `http::error::handle_panic`) end to end. Gated by the
//! `test-routes` Cargo feature, never enabled in the built image.

use axum::routing::get;
use axum::Router;

use super::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/test/panic", get(panic_on_purpose))
}

/// Panics with a fixed, greppable sentinel -- `tests/panic_handling.rs` asserts this
/// exact text never reaches stdout or stderr, which is only true once both
/// `CatchPanicLayer` (the response) and the replaced panic hook (the process's own
/// stderr) are both in place.
async fn panic_on_purpose() {
    panic!("SENTINEL_PANIC");
}
