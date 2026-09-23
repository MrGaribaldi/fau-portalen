#![cfg(feature = "test-routes")]
//! Graceful shutdown (design section 12): SIGTERM sets readiness false before the
//! listener stops accepting, an in-flight request is still allowed to finish, the
//! process exits within the drain bound on a clean shutdown, and a write that
//! cannot commit before that bound elapses is never acknowledged to the client.
//!
//! Needs `/test/slow` and `/test/slow-write`, both gated behind the `test-routes`
//! feature and never present in the built image:
//! `cargo test -p fau-app --features test-routes --test shutdown`.
//! `common::spawn_serve_with_test_routes` is `common::spawn_serve` under another
//! name -- see its doc comment for why an alias is all that is needed.
//!
//! `get_async`'s returned future is lazy -- nothing reaches the wire
//! until it is polled. A test that creates the future, `sleep`s, sends SIGTERM, and
//! only *then* `.await`s it never actually issues the request until after the
//! signal, so a bare sleep before signalling proves nothing about a request being
//! "in flight". Every test below instead polls the request concurrently with
//! sending the signal (`tokio::join!`), and waits for the handler's own
//! started-marker log line (`common::wait_for_log`) before acting on "the request
//! has reached the server" -- proof, not a guess about timing.

mod common;
use common::TestDb;
use std::time::{Duration, Instant};

#[tokio::test]
async fn sigterm_sets_readiness_false_before_the_listener_closes() {
    // Spec section 12: stop taking traffic first, then drain. If the listener
    // closed first, the load balancer would still be sending requests to a closed
    // socket. No in-flight request is involved here, so this one test's SIGTERM can
    // be sent directly rather than raced against anything.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    assert_eq!(app.get("/health/ready").await.status(), 200);

    app.send_sigterm().await;
    let saw_503 = common::poll_until(
        || app.get_async("/health/ready"),
        |r| r.status() == 503,
        Duration::from_secs(3),
    )
    .await;
    assert!(saw_503, "readiness did not go false while still serving");
}

#[tokio::test]
async fn an_in_flight_request_completes_after_sigterm() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_test_routes(&db).await;

    // `tokio::join!` polls both futures concurrently on this task: `/test/slow`'s
    // request future actually starts running (and yields at its own 2-second sleep)
    // as soon as the second future first yields (at `wait_for_log`'s internal
    // sleep), rather than sitting completely unpolled until this whole expression
    // resolves. The second future only sends SIGTERM once `/test/slow`'s own
    // started-marker line has actually appeared in captured stdout -- proof the
    // request reached the handler, not an assumption from elapsed wall-clock time.
    let (res, saw_started) = tokio::join!(app.get_async("/test/slow"), async {
        let saw_started =
            common::wait_for_log(&app, common::SLOW_STARTED_MARKER, Duration::from_secs(3)).await;
        app.send_sigterm().await;
        saw_started
    });

    assert!(
        saw_started,
        "/test/slow never logged its started marker before SIGTERM was sent"
    );
    let res = res.expect("an in-flight request was cut off");
    assert_eq!(res.status(), 200, "an in-flight request was cut off");
}

#[tokio::test]
async fn the_process_exits_within_the_drain_bound() {
    // The idle case (no in-flight request at all) has nothing to drain,
    // so it should finish in well under the 25s bound, not merely under some value
    // close to it -- asserting `< 30s` here would pass even if the drain bound were
    // being applied when it should not be.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let started = Instant::now();
    app.send_sigterm().await;
    let status = app.wait().await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "an idle shutdown took {:?}; nothing was in flight to drain",
        started.elapsed()
    );
    assert!(
        status.success(),
        "clean shutdown must exit 0, got {status:?}"
    );
}

#[tokio::test]
async fn an_aborted_transaction_is_never_acknowledged() {
    // Spec section 12. /test/slow-write opens a transaction, sleeps past the drain
    // bound, then commits. Asserting only `!(acknowledged && rows == 0)` would be
    // too weak -- it also passes if the write was acknowledged *and* committed, or
    // if the request was never actually sent at all.
    // With delay_ms 40000, well past the 25s drain bound, the commit cannot happen,
    // so every part of "the timeout path actually ran and aborted this write" is
    // asserted directly: the client never sees success, the watchdog's own exit
    // code and log line are both present, the elapsed time matches the drain bound
    // rather than some other timeout, and the row is absent.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_test_routes(&db).await;

    let (outcome, signalled_at) =
        tokio::join!(app.get_async("/test/slow-write?delay_ms=40000"), async {
            let saw_started = common::wait_for_log(
                &app,
                common::SLOW_WRITE_STARTED_MARKER,
                Duration::from_secs(3),
            )
            .await;
            assert!(
                saw_started,
                "/test/slow-write never logged its started marker before SIGTERM was sent"
            );
            let signalled_at = Instant::now();
            app.send_sigterm().await;
            signalled_at
        });

    let acknowledged = outcome.map(|r| r.status().is_success()).unwrap_or(false);
    assert!(
        !acknowledged,
        "success was acknowledged for a write that could not commit before the drain bound"
    );

    let status = app.wait().await;
    let drain_took = signalled_at.elapsed();
    assert!(
        drain_took >= Duration::from_millis(24_500),
        "the watchdog fired after only {drain_took:?}; the drain bound is 25s and this write's \
         delay_ms (40000) should have forced the full bound to elapse"
    );
    assert!(
        drain_took < Duration::from_secs(30),
        "the watchdog took {drain_took:?} to fire; the drain bound is 25s"
    );
    // `shutdown::DRAIN_TIMEOUT_EXIT_CODE` -- not importable here, `fau-app` is a
    // binary-only crate with no `lib.rs` a test binary could link against, so this
    // is the same literal by hand.
    assert_eq!(
        status.code(),
        Some(1),
        "expected the drain-timeout exit code, got {status:?}"
    );

    let stdout = app.captured_stdout().join("\n");
    assert!(
        stdout.contains("drain bound elapsed"),
        "no drain-timeout WARN line found in captured stdout:\n{stdout}"
    );

    let rows: i64 = sqlx::query_scalar("select count(*) from tenants where name = 'slow-write'")
        .fetch_one(&db.admin_pool())
        .await
        .unwrap();
    assert_eq!(
        rows, 0,
        "a row committed even though its connection should have been dropped mid-transaction"
    );
}
