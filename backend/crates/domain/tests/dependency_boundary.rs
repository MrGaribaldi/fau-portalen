//! ADR-001: the domain does not depend on HTTP or UI. The design's section 2 makes
//! this a property of the dependency graph rather than of memory, so assert on the
//! manifest itself.

#[test]
fn domain_manifest_declares_no_http_or_sql_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in ["axum", "sqlx", "tower", "hyper", "reqwest"] {
        assert!(
            !manifest.contains(forbidden),
            "domain/Cargo.toml must not declare {forbidden}"
        );
    }
}
