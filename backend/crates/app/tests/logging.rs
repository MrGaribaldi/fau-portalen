//! Structured JSON logging and request context, from the design's section 10 (Alloy's
//! `stage.json` extracts a top-level `level`), section 11 (the request id ties an
//! error body to the log line) and section 15 (sentinel secrets must never appear in
//! log output).

mod common;
use common::TestDb;

#[tokio::test]
async fn every_line_is_json_with_a_conventional_level() {
    // Spec section 10: Alloy's stage.json extracts a top-level `level` and promotes
    // it to a Loki label. A non-JSON line or a renamed field breaks the dashboards.
    // Fix round 1: `timestamp` and `service_version` must be present on *every*
    // line too -- not just ones a call site remembers to pass them on -- so this
    // runs against a path that also emits a WARN with no `AppState` in scope at all
    // (`readiness`'s probe warning, forced by the sentinel-password setup), not just
    // the plain INFO access log from `/health/live`.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_sentinels(&db).await;
    app.get("/health/live").await;
    app.get("/health/ready").await;
    let lines = app.captured_stdout();

    assert!(!lines.is_empty(), "no log output captured");
    let mut saw_warn = false;
    for line in &lines {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON: {line} ({e})"));
        let level = v["level"].as_str().expect("no top-level level field");
        assert!(
            ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"].contains(&level),
            "level was {level}"
        );
        assert!(
            v.get("timestamp").is_some_and(|t| t.is_string()),
            "no top-level timestamp field: {line}"
        );
        assert_eq!(
            v.get("service_version"),
            Some(&serde_json::Value::from(env!("CARGO_PKG_VERSION"))),
            "no top-level service_version field: {line}"
        );
        saw_warn |= level == "WARN";
    }
    assert!(
        saw_warn,
        "expected at least one WARN line (the readiness probe failure) among:\n{}",
        lines.join("\n")
    );
}

#[tokio::test]
async fn the_access_log_uses_the_route_template_not_the_uri() {
    // Spec section 10: the template is what keeps identifiers out of the logs.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    app.get("/api/v1/tenants/0190c3f2-dead-7000-8000-000000000001")
        .await;
    let lines = app.captured_stdout();
    let joined = lines.join("\n");
    assert!(
        !joined.contains("0190c3f2-dead-7000-8000-000000000001"),
        "a path identifier reached the log: {joined}"
    );
}

#[tokio::test]
async fn sentinel_secrets_never_appear_in_log_output() {
    // Spec section 15. The sentinels are planted in exactly the places a leak
    // historically comes from: the DSN password, a query string, a header, a cookie.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_sentinels(&db).await;

    app.get("/?token=SENTINEL_QUERY").await;
    app.get_with_headers(
        "/health/live",
        &[
            ("authorization", "Bearer SENTINEL_AUTHZ"),
            ("cookie", "session=SENTINEL_COOKIE"),
            ("x-email", "SENTINEL_EMAIL@example.test"),
        ],
    )
    .await;
    app.get("/api/v1/nope?q=SENTINEL_QUERY2").await;

    // Fix round 1: prove the connection-failure path this sentinel is meant to
    // exercise actually ran, rather than trusting that the harness helper's own
    // internal `/health/ready` call did -- a wrong DSN password must surface as a
    // database problem, not silently as "ready".
    let ready = app.get("/health/ready").await;
    assert_eq!(ready.status(), 503);
    let body: serde_json::Value = ready.json().await.expect("a JSON readiness body");
    assert_eq!(body["reason"], "database");

    // Fix round 1: check stdout *and* stderr -- a leak on either stream is a leak.
    let stdout = app.captured_stdout().join("\n");
    let stderr = app.captured_stderr().join("\n");
    for sentinel in [
        "SENTINEL_DB_PASSWORD", // planted inside DATABASE_URL
        "SENTINEL_QUERY",
        "SENTINEL_QUERY2",
        "SENTINEL_AUTHZ",
        "SENTINEL_COOKIE",
        "SENTINEL_EMAIL",
    ] {
        assert!(
            !stdout.contains(sentinel),
            "{sentinel} reached stdout:\n{stdout}"
        );
        assert!(
            !stderr.contains(sentinel),
            "{sentinel} reached stderr:\n{stderr}"
        );
    }
}

#[tokio::test]
async fn the_request_id_is_echoed_and_logged() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/health/live").await;
    let id = res.headers()["x-request-id"].to_str().unwrap().to_string();
    assert!(!id.is_empty());
    let captured = app.captured_stdout().join("\n");
    assert!(captured.contains(&id), "request id {id} not in the log");
}

#[tokio::test]
async fn inbound_request_id_header_is_never_trusted() {
    // Ruling 5 (Task 10) and fix round 1, item 3: a caller-supplied `x-request-id`
    // must never be echoed back, never appear in a log line, and never substitute
    // for the id this server mints -- only the id `middleware` generates may tie the
    // response, the error body and the access-log line together.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;

    let res = app
        .get_with_headers("/api/v1/nope", &[("x-request-id", "SENTINEL_RID")])
        .await;
    let response_id = res.headers()["x-request-id"].to_str().unwrap().to_string();
    assert_ne!(
        response_id, "SENTINEL_RID",
        "an inbound x-request-id must never be echoed back"
    );

    let no_leaked_header = !res
        .headers()
        .values()
        .any(|v| v.to_str().unwrap_or_default().contains("SENTINEL_RID"));

    let body: serde_json::Value = res.json().await.expect("a JSON error body");
    assert_eq!(
        body["request_id"], response_id,
        "the error body's request_id must match the response header"
    );

    let lines = app.captured_stdout();
    let access_line = lines
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|v| {
            v.get("request_id")
                .and_then(|id| id.as_str())
                .is_some_and(|id| id == response_id)
                && v.get("route").is_some()
        })
        .unwrap_or_else(|| {
            panic!(
                "no access-log line carrying request id {response_id} found in:\n{}",
                lines.join("\n")
            )
        });
    assert_eq!(access_line["route"], "<unmatched>");

    let joined = lines.join("\n");
    let stderr = app.captured_stderr().join("\n");
    assert!(
        !joined.contains("SENTINEL_RID"),
        "the inbound request id leaked into stdout: {joined}"
    );
    assert!(
        !stderr.contains("SENTINEL_RID"),
        "the inbound request id leaked into stderr: {stderr}"
    );
    assert!(
        no_leaked_header,
        "the inbound request id leaked into a response header"
    );
}

#[tokio::test]
async fn the_warn_line_inside_a_request_carries_its_request_id_even_at_log_level_warn() {
    // Fix round 2, item 1: an `info_span!`-level span is disabled outright by
    // `EnvFilter` at `LOG_LEVEL=warn` (or stricter), so a `WARN` line nested inside
    // it -- like `readiness`'s probe failure below -- would lose `request_id`
    // entirely if the per-request span itself were ever filtered out along with it.
    // `telemetry::REQUEST_SPAN_TARGET` gives that span its own always-on directive
    // specifically so this cannot happen, independent of `LOG_LEVEL`.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_sentinels_and_env(&db, &[("LOG_LEVEL", "warn")]).await;

    let res = app.get("/health/ready").await;
    assert_eq!(res.status(), 503);
    let request_id = res.headers()["x-request-id"].to_str().unwrap().to_string();

    let lines = app.captured_stdout();
    let warn_line = lines
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        // Matched by *this* request's id specifically, not just "any WARN with a
        // request_id": the harness helper's own internal `/health/ready` call (and,
        // at LOG_LEVEL=warn, the startup schema-contract retry warning with no
        // request in flight at all) produce other WARN lines first.
        .find(|v| {
            v["level"] == "WARN"
                && v.get("request_id").and_then(|id| id.as_str()) == Some(request_id.as_str())
        })
        .unwrap_or_else(|| {
            panic!(
                "no WARN line carrying request id {request_id} found in stdout:\n{}",
                lines.join("\n")
            )
        });
    assert_eq!(
        warn_line["request_id"], request_id,
        "the readiness WARN line must carry this request's id even at LOG_LEVEL=warn: {warn_line}"
    );
}

#[tokio::test]
async fn sqlx_query_text_never_appears_in_log_output_even_at_log_level_debug() {
    // Spec section 10: "never logged: ... SQL parameters" -- more basically, sqlx's
    // own query logging must not leak SQL text at all. Fix round 2, item 5: at the
    // *default* LOG_LEVEL=info, sqlx's own statement logging (DEBUG by default)
    // would already be suppressed by the global level alone, so this test could
    // pass even if the dedicated `sqlx=warn` directive in `telemetry::init` were
    // accidentally removed -- it would not actually be guarding anything.
    // LOG_LEVEL=debug is the level that makes it a real guard: without `sqlx=warn`,
    // sqlx's statement logs would pass the (now debug) global filter.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_env(&db, &[("LOG_LEVEL", "debug")]).await;
    app.get("/health/ready").await;

    let captured = app.captured_stdout().join("\n").to_lowercase();
    for needle in ["select", "db.statement"] {
        assert!(
            !captured.contains(needle),
            "sqlx query text ({needle}) reached stdout at LOG_LEVEL=debug:\n{captured}"
        );
    }
}

#[test]
fn no_log_is_exported_over_otlp() {
    // Spec section 10: Alloy has an OTLP receiver, so exporting logs there too
    // would double-collect every line. OTLP is reserved for traces.
    let manifest = include_str!("../Cargo.toml");
    assert!(
        !manifest.contains("opentelemetry-appender"),
        "an OTLP log appender was added; spec section 10 forbids it"
    );
}
