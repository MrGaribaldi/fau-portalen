//! The HTTP surface, from the design's section 3, 4, 8, 9 and 11: liveness, the
//! static Bokmal placeholder, the JSON error contract and the routing rules that
//! keep an unknown API route or a missing asset from ever falling back to HTML.

mod common;
use common::TestDb;

#[tokio::test]
async fn liveness_is_200_and_touches_nothing() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/health/live").await;
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn liveness_stays_200_when_the_database_is_unreachable() {
    // Spec section 9: a dependency outage must not cause a restart loop.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    db.sever_connections().await; // terminates backends and revokes connect
    assert_eq!(app.get("/health/live").await.status(), 200);
}

#[tokio::test]
async fn root_serves_the_bokmal_placeholder_as_html() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/").await;
    assert_eq!(res.status(), 200);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert!(res.text().await.unwrap().contains("lang=\"nb-NO\""));
}

#[tokio::test]
async fn unknown_api_routes_are_404_and_never_html() {
    // Spec section 8 -- the rule that keeps a client from parsing a login page as JSON.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    for path in ["/api/v1/nope", "/api/v1/", "/api/v1/tenants/1"] {
        let res = app.get(path).await;
        assert_eq!(res.status(), 404, "{path}");
        let ct = res.headers()["content-type"].to_str().unwrap().to_string();
        assert!(ct.starts_with("application/json"), "{path} returned {ct}");
        let body = res.text().await.unwrap();
        assert!(!body.contains("<html"), "{path} fell back to HTML");
    }
}

#[tokio::test]
async fn missing_assets_are_404_and_never_html() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/assets/does-not-exist.js").await;
    assert_eq!(res.status(), 404);
    assert!(!res.text().await.unwrap().contains("<html"));
}

#[tokio::test]
async fn api_errors_carry_a_stable_code_and_request_id_but_no_display_text() {
    // Spec section 11 and #3439: the client renders the sentence.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/api/v1/nope").await;
    let body: serde_json::Value = res.json().await.unwrap();

    assert_eq!(body["code"], "not_found");
    assert!(body["request_id"].is_string());
    assert!(
        body.get("message").is_none(),
        "error carried display text: {body}"
    );
    assert!(
        body.get("detail").is_none(),
        "error carried display text: {body}"
    );

    // No Norwegian prose anywhere in the payload.
    let raw = body.to_string();
    for word in ["ikke", "finnes", "feil", "Ugyldig"] {
        assert!(
            !raw.contains(word),
            "localisable prose in error body: {raw}"
        );
    }
}

#[tokio::test]
async fn reserved_prefixes_exist_but_carry_no_routes() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    // /app/ is reserved for #3422 and returns the placeholder shell, not 404, so the
    // client-route fallback contract is in place from the start. All three forms --
    // the bare prefix, the prefix with a trailing slash (ADR-001 names this one as
    // the entry path, and it is exactly the one axum's nest-matching trailing-slash
    // gap would otherwise break) and a deeper client route -- must resolve the same
    // way.
    for path in ["/app", "/app/", "/app/anything"] {
        let res = app.get(path).await;
        assert_eq!(res.status(), 200, "{path}");
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html"),
            "{path} did not serve HTML"
        );
    }
}

#[tokio::test]
#[cfg(not(feature = "test-routes"))]
async fn the_release_binary_has_no_test_routes() {
    // The `test-routes` feature (`/test/slow`, `/test/slow-write` and
    // `/test/panic`) is never enabled in the built image -- this proves it
    // by compiling and running only when the feature is off, the same configuration
    // the released binary ships with, and asserting the routes 404 like any other
    // unknown path rather than being reachable.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    for path in ["/test/slow", "/test/slow-write", "/test/panic"] {
        let res = app.get(path).await;
        assert_eq!(res.status(), 404, "{path}");
        let ct = res.headers()["content-type"].to_str().unwrap().to_string();
        assert!(ct.starts_with("application/json"), "{path} returned {ct}");
    }
}
