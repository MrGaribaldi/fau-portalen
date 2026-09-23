#![cfg(feature = "test-routes")]
//! End-to-end proof of the panic-catching path (design section 11, fix round 2 item
//! 3): a real `tower_http::CatchPanicLayer`, the process-wide panic hook
//! `telemetry::init` installs, and `http::error::handle_panic`, all wired together
//! exactly as `fau serve` runs them -- not just `handle_panic` called directly, which
//! `http::error`'s own unit test already covers in isolation.
//!
//! Requires the `test-routes` feature, which registers `/test/panic` (never present
//! in the built image): `cargo test -p fau-app --features test-routes --test panic_handling`.

mod common;
use common::TestDb;

#[tokio::test]
async fn a_handler_panic_is_caught_and_answered_with_the_json_error_contract() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;

    let res = app.get("/test/panic").await;
    assert_eq!(res.status(), 500);
    let request_id = res.headers()["x-request-id"].to_str().unwrap().to_string();

    let body: serde_json::Value = res.json().await.expect("a JSON error body");
    assert_eq!(body["code"], "internal_error");
    assert_eq!(body["request_id"], request_id);

    let stdout_lines = app.captured_stdout();
    let stdout = stdout_lines.join("\n");
    let stderr = app.captured_stderr().join("\n");

    let has_error_line = stdout_lines
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(|v| {
            v["level"] == "ERROR"
                && v.get("request_id").and_then(|id| id.as_str()) == Some(request_id.as_str())
        });
    assert!(
        has_error_line,
        "no JSON ERROR line carrying request id {request_id} found in stdout:\n{stdout}"
    );

    // Without the process-wide panic hook `telemetry::init` installs, Rust's
    // default hook would print `thread '...' panicked at ...: SENTINEL_PANIC`
    // straight to stderr, bypassing the JSON transport and the whole reason this
    // sentinel exists.
    assert!(
        !stdout.contains("SENTINEL_PANIC"),
        "the panic payload leaked into stdout: {stdout}"
    );
    assert!(
        !stderr.contains("SENTINEL_PANIC"),
        "the panic payload leaked into stderr: {stderr}"
    );
}
