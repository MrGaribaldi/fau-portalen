//! Routes that exist only so an integration test can exercise behaviour that is
//! hard to trigger through the ordinary HTTP surface: a deliberate handler panic
//! (`tests/panic_handling.rs`, proving the whole panic-catching path -- a real
//! `CatchPanicLayer`, the process-wide panic hook installed by `telemetry::init`,
//! `http::error::handle_panic` -- end to end), and two routes `tests/shutdown.rs`
//! uses to hold a connection open across a SIGTERM: a plain slow handler for an
//! in-flight request that must be allowed to finish, and a slow handler that holds
//! an open transaction, for a write that must *not* be allowed to finish once the
//! drain bound elapses. Gated by the `test-routes` Cargo feature, never enabled in
//! the built image (`tests/http_surface.rs`'s `the_release_binary_has_no_test_routes`
//! enforces that from the other side).

use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use uuid::Uuid;

use super::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/test/panic", get(panic_on_purpose))
        .route("/test/slow", get(slow))
        .route("/test/slow-write", get(slow_write))
}

/// Panics with a fixed, greppable sentinel -- `tests/panic_handling.rs` asserts this
/// exact text never reaches stdout or stderr, which is only true once both
/// `CatchPanicLayer` (the response) and the replaced panic hook (the process's own
/// stderr) are both in place.
async fn panic_on_purpose() {
    panic!("SENTINEL_PANIC");
}

/// Sleeps 2 seconds, then returns 200. Long enough for
/// `tests/shutdown.rs::an_in_flight_request_completes_after_sigterm` to send SIGTERM
/// while this request is still running, and short enough to stay well inside the
/// 25-second drain bound so the test itself runs fast.
///
/// The `INFO` line logged first, synchronously, before the sleep, is this route's
/// started-marker: `tests/common/mod.rs::SLOW_STARTED_MARKER` polls for its exact
/// text in captured stdout, which is how the test proves the request actually
/// reached this handler before it acts on that fact (e.g. sends SIGTERM), rather
/// than guessing from a fixed sleep how long a lazy, not-yet-polled request future
/// takes to reach the wire. Keep this text and that constant identical by hand --
/// see the constant's own doc comment for why they cannot be one shared value.
async fn slow() -> StatusCode {
    tracing::info!("test_routes: /test/slow started");
    tokio::time::sleep(Duration::from_secs(2)).await;
    StatusCode::OK
}

#[derive(Deserialize)]
struct SlowWriteParams {
    delay_ms: u64,
}

/// Opens a transaction on the runtime pool, inserts a sentinel school and a tenant
/// named `slow-write` pointed at it, sleeps `delay_ms`, then commits and returns
/// 200.
///
/// `tests/shutdown.rs::an_aborted_transaction_is_never_acknowledged` calls this with
/// a `delay_ms` far past the 25-second drain bound: the drain-bound watchdog
/// (`crate::shutdown`) force-exits the process while this handler is still asleep,
/// which drops its connection -- including the still-open transaction -- before
/// `commit` is ever reached. PostgreSQL rolls back a transaction whose connection
/// disappears without a commit, so neither row this handler inserted ever actually
/// lands, and the client never receives this handler's `200` either, since the
/// connection carrying the response is gone too. That is what makes "an aborted
/// transaction is never acknowledged" true of the real drain path, not just of this
/// handler's own code.
///
/// `state.pool` runs as the runtime role `fau_app` (`compose.yaml`'s `DATABASE_URL`),
/// and `tenants.school_id` (#3441, migration 0004) is `not null` with a foreign key
/// to `schools`. `fau_app` may insert only a fresh, pending, submitted school (0004's
/// row-level security), which itself needs an existing `municipality_id` --
/// `fau_app` has no insert grant on `municipalities` at all, so this route borrows
/// whichever municipality the test harness has already seeded
/// (`tests/shutdown.rs` seeds one before spawning `serve`) rather than creating one.
///
/// Logs its own started-marker first, synchronously, before opening the
/// transaction -- see [`slow`]'s doc comment for why this exists and
/// `tests/common/mod.rs::SLOW_WRITE_STARTED_MARKER` for the constant that must stay
/// identical to it. Deliberately does not log `delay_ms`: it is a query-string
/// value, and design section 10 never logs those, even though this particular one
/// carries nothing sensitive.
async fn slow_write(
    State(state): State<AppState>,
    Query(params): Query<SlowWriteParams>,
) -> StatusCode {
    tracing::info!("test_routes: /test/slow-write started");

    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin transaction for /test/slow-write");

    let municipality_id: Uuid = sqlx::query_scalar("select id from municipalities limit 1")
        .fetch_one(&mut *tx)
        .await
        .expect(
            "a municipality must already exist for /test/slow-write's sentinel school \
             -- fau_app cannot create one itself",
        );

    let school_id = Uuid::now_v7();
    sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, verification, status, search_text)
         values ($1, $2, 'submitted', 'slow-write', 'pending', 'active', 'slow-write')",
    )
    .bind(school_id)
    .bind(municipality_id)
    .execute(&mut *tx)
    .await
    .expect("insert sentinel school for /test/slow-write");

    sqlx::query(
        "insert into tenants (id, name, status, school_id) values ($1, 'slow-write', 'pending', $2)",
    )
    .bind(Uuid::now_v7())
    .bind(school_id)
    .execute(&mut *tx)
    .await
    .expect("insert sentinel tenant for /test/slow-write");

    tokio::time::sleep(Duration::from_millis(params.delay_ms)).await;

    tx.commit()
        .await
        .expect("commit /test/slow-write transaction");

    StatusCode::OK
}
