//! Configuration loading and validation, from the design's section 4.
//!
//! The rule under test throughout: an invalid or missing required variable fails at
//! startup with the *variable name, never its value*, and a non-zero exit.

use std::process::Command;

const SERVE_ENV: &[(&str, &str)] = &[
    ("APP_ENV", "test"),
    ("HTTP_BIND", "127.0.0.1:0"),
    ("PUBLIC_BASE_URL", "http://localhost:8000"),
    ("DATABASE_URL", "postgres://u:hunter2@127.0.0.1:1/fau"),
    ("DB_POOL_MAX_CONNECTIONS", "5"),
    ("LOG_LEVEL", "info"),
];

fn fau_with(env: &[(&str, &str)], arg: &str) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fau"));
    cmd.arg(arg)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("run fau")
}

fn serve_env_without(name: &str) -> Vec<(&'static str, &'static str)> {
    SERVE_ENV
        .iter()
        .copied()
        .filter(|(k, _)| *k != name)
        .collect()
}

fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn missing_required_variable_names_it_and_exits_non_zero() {
    let out = fau_with(&serve_env_without("PUBLIC_BASE_URL"), "serve");
    assert!(!out.status.success());
    let text = combined(&out);
    assert!(text.contains("PUBLIC_BASE_URL"), "output was: {text}");
}

#[test]
fn invalid_value_is_never_echoed() {
    let mut env = serve_env_without("DB_POOL_MAX_CONNECTIONS");
    env.push(("DB_POOL_MAX_CONNECTIONS", "banana-sentinel"));
    let out = fau_with(&env, "serve");
    assert!(!out.status.success());
    let text = combined(&out);
    assert!(
        text.contains("DB_POOL_MAX_CONNECTIONS"),
        "output was: {text}"
    );
    assert!(!text.contains("banana-sentinel"), "value leaked: {text}");
}

#[test]
fn zero_pool_connections_is_rejected() {
    let mut env = serve_env_without("DB_POOL_MAX_CONNECTIONS");
    env.push(("DB_POOL_MAX_CONNECTIONS", "0"));
    let out = fau_with(&env, "serve");
    assert!(
        !out.status.success(),
        "a pool of zero connections is unusable"
    );
    assert!(combined(&out).contains("DB_POOL_MAX_CONNECTIONS"));
}

#[test]
fn unknown_app_env_is_rejected_without_echoing_it() {
    let mut env = serve_env_without("APP_ENV");
    env.push(("APP_ENV", "staging-sentinel"));
    let out = fau_with(&env, "serve");
    assert!(!out.status.success());
    let text = combined(&out);
    assert!(text.contains("APP_ENV"), "output was: {text}");
    assert!(!text.contains("staging-sentinel"), "value leaked: {text}");
}

#[test]
fn production_requires_an_https_public_base_url() {
    let mut env = serve_env_without("APP_ENV");
    env.retain(|(k, _)| *k != "PUBLIC_BASE_URL");
    env.push(("APP_ENV", "production"));
    env.push(("PUBLIC_BASE_URL", "http://fau.example"));
    let out = fau_with(&env, "serve");
    assert!(
        !out.status.success(),
        "production accepted a plaintext base URL"
    );
    assert!(combined(&out).contains("PUBLIC_BASE_URL"));
}

#[test]
fn database_url_password_never_appears_in_output() {
    // A DSN in an error message is the classic way a password reaches a log.
    let out = fau_with(&serve_env_without("PUBLIC_BASE_URL"), "serve");
    assert!(!combined(&out).contains("hunter2"), "password leaked");
}

#[test]
fn migrate_does_not_require_serve_configuration() {
    // Section 4: migrate requires only MIGRATION_DATABASE_URL. With a syntactically
    // valid one supplied, nothing here may fail on a *serve* variable -- today that
    // shows up as `migrate`'s `todo!()`, later as a connection failure, but never as
    // a configuration error naming APP_ENV, PUBLIC_BASE_URL, DATABASE_URL,
    // DB_POOL_MAX_CONNECTIONS, HTTP_BIND or LOG_LEVEL.
    let out = fau_with(
        &[("MIGRATION_DATABASE_URL", "postgres://u:p@127.0.0.1:1/none")],
        "migrate",
    );
    assert!(!out.status.success());
    let text = combined(&out);
    for serve_var in [
        "APP_ENV",
        "PUBLIC_BASE_URL",
        "DATABASE_URL",
        "DB_POOL_MAX_CONNECTIONS",
        "HTTP_BIND",
        "LOG_LEVEL",
    ] {
        assert!(
            !text.contains(serve_var),
            "output named serve variable {serve_var}: {text}"
        );
    }
    assert!(
        !text.contains("configuration variable"),
        "output was: {text}"
    );
}

#[test]
fn telemetry_and_mail_variables_are_accepted_without_validation() {
    // Spec section 4: these have no adapter yet, so any value -- even nonsense --
    // must never produce a configuration error naming them.
    let mut env = SERVE_ENV.to_vec();
    env.push(("OTEL_SERVICE_NAME", "fau-app"));
    env.push(("OTEL_EXPORTER_OTLP_ENDPOINT", "not-a-real-endpoint"));
    env.push(("OTEL_EXPORTER_OTLP_PROTOCOL", "smoke-signal"));
    env.push(("MAIL_TRANSPORT", "carrier-pigeon"));
    env.push(("SIGNUP_NOTIFICATION_TO", "not-an-email-address"));
    let out = fau_with(&env, "serve");
    let text = combined(&out);
    assert!(
        !text.contains("configuration variable"),
        "output was: {text}"
    );
}

#[test]
fn migrate_without_its_own_variable_names_it() {
    let out = fau_with(&[], "migrate");
    assert!(!out.status.success());
    assert!(combined(&out).contains("MIGRATION_DATABASE_URL"));
}

#[test]
fn http_bind_and_log_level_have_defaults() {
    let mut env = serve_env_without("HTTP_BIND");
    env.retain(|(k, _)| *k != "LOG_LEVEL");
    let out = fau_with(&env, "serve");
    let text = combined(&out);
    assert!(!text.contains("HTTP_BIND"), "output was: {text}");
    assert!(!text.contains("LOG_LEVEL"), "output was: {text}");
}

// The migration lock's bounds, ruling 7: MIGRATION_LOCK_TIMEOUT_MS and
// MIGRATION_LOCK_WAIT_MS default when absent, and an invalid value is rejected by
// variable name only. The database in MIGRATION_DATABASE_URL is deliberately
// unreachable (127.0.0.1:1) in all three: config validation happens before any
// connection attempt, so these never depend on the runner actually connecting.

#[test]
fn migrate_lock_timeout_and_wait_have_defaults() {
    let out = fau_with(
        &[("MIGRATION_DATABASE_URL", "postgres://u:p@127.0.0.1:1/none")],
        "migrate",
    );
    let text = combined(&out);
    assert!(
        !text.contains("MIGRATION_LOCK_TIMEOUT_MS"),
        "output was: {text}"
    );
    assert!(
        !text.contains("MIGRATION_LOCK_WAIT_MS"),
        "output was: {text}"
    );
}

#[test]
fn migrate_rejects_an_invalid_lock_timeout_without_echoing_it() {
    let out = fau_with(
        &[
            ("MIGRATION_DATABASE_URL", "postgres://u:p@127.0.0.1:1/none"),
            ("MIGRATION_LOCK_TIMEOUT_MS", "banana-sentinel"),
        ],
        "migrate",
    );
    assert!(!out.status.success());
    let text = combined(&out);
    assert!(
        text.contains("MIGRATION_LOCK_TIMEOUT_MS"),
        "output was: {text}"
    );
    assert!(!text.contains("banana-sentinel"), "value leaked: {text}");
}

#[test]
fn migrate_rejects_an_invalid_lock_wait_without_echoing_it() {
    let out = fau_with(
        &[
            ("MIGRATION_DATABASE_URL", "postgres://u:p@127.0.0.1:1/none"),
            ("MIGRATION_LOCK_WAIT_MS", "banana-sentinel"),
        ],
        "migrate",
    );
    assert!(!out.status.success());
    let text = combined(&out);
    assert!(
        text.contains("MIGRATION_LOCK_WAIT_MS"),
        "output was: {text}"
    );
    assert!(!text.contains("banana-sentinel"), "value leaked: {text}");
}
