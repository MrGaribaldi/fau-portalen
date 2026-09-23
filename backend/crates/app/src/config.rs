//! Typed configuration, from the design's section 4.
//!
//! One rule governs this module: an invalid or missing required variable fails at
//! startup with the **variable name, never its value**, and a non-zero exit. Every
//! read goes through [`required`] or [`optional`] so that rule cannot be forgotten
//! one variable at a time.
//!
//! A database that is merely unreachable is *not* a configuration error. It does not
//! appear here at all -- readiness handles it, because a restart loop during a brief
//! database outage turns a recoverable incident into an outage of our own.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use tracing::level_filters::LevelFilter;
use url::Url;

/// A value that must never be printed. `Debug` and `Display` both redact, so a
/// `{:?}` on a struct holding one cannot leak it either.
#[derive(Clone)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Named so that every place a secret is read is greppable.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigProblem {
    Missing,
    Empty,
    Invalid,
    /// The value parsed but is not allowed in this environment.
    NotPermitted,
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Missing => "missing",
            Self::Empty => "empty",
            Self::Invalid => "not a valid value",
            Self::NotPermitted => "not permitted in this environment",
        };
        f.write_str(text)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigError {
    pub variable: &'static str,
    pub problem: ConfigProblem,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "configuration variable {} is {}",
            self.variable, self.problem
        )
    }
}

impl std::error::Error for ConfigError {}

fn err(variable: &'static str, problem: ConfigProblem) -> ConfigError {
    ConfigError { variable, problem }
}

fn optional(name: &'static str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

fn required(name: &'static str) -> Result<String, ConfigError> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        Ok(_) => Err(err(name, ConfigProblem::Empty)),
        Err(_) => Err(err(name, ConfigProblem::Missing)),
    }
}

/// The parse error is discarded on purpose: several `FromStr` implementations quote
/// the offending input, which is exactly what must not reach the output.
fn parsed<T: FromStr>(name: &'static str, raw: &str) -> Result<T, ConfigError> {
    raw.trim()
        .parse()
        .map_err(|_| err(name, ConfigProblem::Invalid))
}

/// Validates a PostgreSQL connection URL, failing by variable name alone. The scheme
/// is checked separately because sqlx's own parser accepts any scheme, and the value
/// is kept as a [`Secret`] string rather than the parsed options, whose `Debug`
/// prints the password.
fn postgres_url(name: &'static str, raw: String) -> Result<Secret<String>, ConfigError> {
    let invalid = || err(name, ConfigProblem::Invalid);
    let url = Url::parse(raw.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "postgres" | "postgresql") {
        return Err(invalid());
    }
    sqlx::postgres::PgConnectOptions::from_str(raw.trim()).map_err(|_| invalid())?;
    Ok(Secret::new(raw))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnv {
    Development,
    Test,
    Production,
}

impl AppEnv {
    pub fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

impl FromStr for AppEnv {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "development" => Ok(Self::Development),
            "test" => Ok(Self::Test),
            "production" => Ok(Self::Production),
            _ => Err(()),
        }
    }
}

impl fmt::Display for AppEnv {
    /// The exact configured string, round-tripped through `FromStr`'s own match arms
    /// -- so the JSON startup event logs `"development"`, not `Debug`'s
    /// `"Development"`. `#[derive(Debug)]`'s casing is a Rust-ism a log consumer
    /// (and design section 4's own vocabulary) has no reason to expect.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Development => "development",
            Self::Test => "test",
            Self::Production => "production",
        };
        f.write_str(text)
    }
}

#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub app_env: AppEnv,
    pub http_bind: SocketAddr,
    pub public_base_url: Url,
    pub database_url: Secret<String>,
    pub db_pool_max_connections: u32,
    pub log_level: String,
    /// Names of the telemetry and mail variables from ADR-001's table that were
    /// set but have no adapter yet. Per spec §4 they are recognised and accepted,
    /// not validated or used until then. Only the names are kept: a value such as
    /// an OTLP endpoint can carry credentials.
    pub accepted_but_unused: Vec<&'static str>,
}

const DEFAULT_HTTP_BIND: &str = "0.0.0.0:8000";
const DEFAULT_LOG_LEVEL: &str = "info";

/// Telemetry and mail variables from ADR-001's table (docs/repo-container-contract.md):
/// accepted and unused until their adapters arrive (spec §4).
const ACCEPTED_BUT_UNUSED: &[&str] = &[
    "OTEL_SERVICE_NAME",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_PROTOCOL",
    "MAIL_TRANSPORT",
    "SIGNUP_NOTIFICATION_TO",
];

impl ServeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let app_env: AppEnv = parsed("APP_ENV", &required("APP_ENV")?)?;

        let http_bind: SocketAddr = parsed(
            "HTTP_BIND",
            &optional("HTTP_BIND").unwrap_or_else(|| DEFAULT_HTTP_BIND.to_owned()),
        )?;

        let raw_base_url = required("PUBLIC_BASE_URL")?;
        let public_base_url: Url = Url::parse(raw_base_url.trim())
            .map_err(|_| err("PUBLIC_BASE_URL", ConfigProblem::Invalid))?;
        validate_public_base_url(&public_base_url, app_env)?;

        let database_url = postgres_url("DATABASE_URL", required("DATABASE_URL")?)?;

        let db_pool_max_connections: u32 = parsed(
            "DB_POOL_MAX_CONNECTIONS",
            &required("DB_POOL_MAX_CONNECTIONS")?,
        )?;
        if db_pool_max_connections == 0 {
            return Err(err("DB_POOL_MAX_CONNECTIONS", ConfigProblem::Invalid));
        }

        let log_level_raw = optional("LOG_LEVEL").unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned());
        // Validated as a `LevelFilter` -- not the fuller `tracing_subscriber::EnvFilter`
        // grammar `telemetry::init` builds from it -- because `EnvFilter` treats an
        // unrecognised bare word as a *target* name (matching every level for it)
        // rather than rejecting it, so a typo such as `banana-sentinel`
        // would silently be accepted as "log everything from a crate named
        // banana-sentinel" instead of failing loudly. `LevelFilter::from_str` has no
        // such fallback: it only accepts trace/debug/info/warn/error/off (or 0-5),
        // which is all `LOG_LEVEL` is meant to carry. `parsed` (this module's helper)
        // discards the parse error itself, since `EnvFilter`'s own parse errors quote
        // the offending input, and a configuration value must never reach the output.
        parsed::<LevelFilter>("LOG_LEVEL", &log_level_raw)?;
        // Stored *trimmed*, not the raw value `parsed` validated: `parsed` trims
        // before parsing (`raw.trim().parse()`), so a value like `"info "` passes
        // validation, but `EnvFilter`'s own grammar does not tolerate the same
        // trailing whitespace the same way -- an untrimmed bare word that fails
        // `LevelFilter::from_str` is read by `EnvFilter` as a *target name* enabling
        // only that one bogus target, not as the global default level, which
        // silently turns off ordinary application logging. Trimming here, once,
        // keeps `telemetry::init` (which builds the actual `EnvFilter` from this
        // field) working from the same value that was actually validated.
        let log_level = log_level_raw.trim().to_owned();

        // Read but never validated, never used -- see ACCEPTED_BUT_UNUSED's doc
        // comment. A missing or garbage value here must not fail startup.
        let accepted_but_unused: Vec<&'static str> = ACCEPTED_BUT_UNUSED
            .iter()
            .copied()
            .filter(|&name| optional(name).is_some())
            .collect();

        Ok(Self {
            app_env,
            http_bind,
            public_base_url,
            database_url,
            db_pool_max_connections,
            log_level,
            accepted_but_unused,
        })
    }
}

fn validate_public_base_url(url: &Url, app_env: AppEnv) -> Result<(), ConfigError> {
    let host = url.host_str().unwrap_or_default();
    // Userinfo in a base URL is a credential that would be logged with the
    // startup event and repeated in every generated link.
    if host.is_empty() || !url.username().is_empty() || url.password().is_some() {
        return Err(err("PUBLIC_BASE_URL", ConfigProblem::Invalid));
    }

    match url.scheme() {
        "https" => Ok(()),
        // ADR-001: HTTPS in production, full stop -- explicit localhost is a
        // development/test convenience and must not carry over to production.
        "http" if !app_env.is_production() && is_loopback(host) => Ok(()),
        _ => Err(err("PUBLIC_BASE_URL", ConfigProblem::NotPermitted)),
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1"
}

#[derive(Debug, Clone)]
pub struct MigrateConfig {
    pub migration_database_url: Secret<String>,
    /// Bounds a single DDL statement's wait on a table lock held by live traffic --
    /// short, because it protects production queries from a stuck migration. Also
    /// bounds sqlx's own internal advisory lock as a backstop, which is exactly why
    /// this cannot double as the run-level queueing wait below; see
    /// `fau_persistence::migrate`'s module docs for why the two must differ.
    pub lock_timeout_ms: u64,
    /// How long it is normal to queue behind another FAU migrator already running
    /// (e.g. during a rolling deploy) before giving up on the outer lock -- long,
    /// unlike `lock_timeout_ms` above.
    pub lock_wait_ms: u64,
}

const DEFAULT_LOCK_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_LOCK_WAIT_MS: u64 = 30_000;

impl MigrateConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let migration_database_url = postgres_url(
            "MIGRATION_DATABASE_URL",
            required("MIGRATION_DATABASE_URL")?,
        )?;

        let lock_timeout_ms = match optional("MIGRATION_LOCK_TIMEOUT_MS") {
            Some(raw) => parsed("MIGRATION_LOCK_TIMEOUT_MS", &raw)?,
            None => DEFAULT_LOCK_TIMEOUT_MS,
        };
        let lock_wait_ms = match optional("MIGRATION_LOCK_WAIT_MS") {
            Some(raw) => parsed("MIGRATION_LOCK_WAIT_MS", &raw)?,
            None => DEFAULT_LOCK_WAIT_MS,
        };

        Ok(Self {
            migration_database_url,
            lock_timeout_ms,
            lock_wait_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_redacts_in_both_debug_and_display() {
        let s = Secret::new("hunter2".to_owned());
        assert_eq!(format!("{s}"), "[redacted]");
        assert_eq!(format!("{s:?}"), "[redacted]");
        assert!(!format!("{s:?}").contains("hunter2"));
    }

    #[test]
    fn config_error_names_the_variable_and_carries_no_value() {
        let e = err("DATABASE_URL", ConfigProblem::Invalid);
        let text = e.to_string();
        assert!(text.contains("DATABASE_URL"));
        assert_eq!(
            text,
            "configuration variable DATABASE_URL is not a valid value"
        );
    }

    #[test]
    fn migrate_config_lock_timeout_and_wait_default_to_10s_and_30s() {
        // The only test in this binary that touches these three variable names, so
        // mutating the process environment here cannot race another test reading
        // the same names.
        unsafe {
            std::env::set_var("MIGRATION_DATABASE_URL", "postgres://u:p@127.0.0.1:1/none");
            std::env::remove_var("MIGRATION_LOCK_TIMEOUT_MS");
            std::env::remove_var("MIGRATION_LOCK_WAIT_MS");
        }

        let cfg = MigrateConfig::from_env().expect("valid minimal migrate config");
        assert_eq!(cfg.lock_timeout_ms, 10_000);
        assert_eq!(cfg.lock_wait_ms, 30_000);

        unsafe {
            std::env::remove_var("MIGRATION_DATABASE_URL");
        }
    }

    #[test]
    fn production_rejects_a_plaintext_base_url() {
        let url = Url::parse("http://fau.example").unwrap();
        assert!(validate_public_base_url(&url, AppEnv::Production).is_err());
    }

    #[test]
    fn production_accepts_https() {
        let url = Url::parse("https://fau.example").unwrap();
        assert!(validate_public_base_url(&url, AppEnv::Production).is_ok());
    }

    #[test]
    fn development_accepts_plaintext_localhost() {
        let url = Url::parse("http://localhost:8000").unwrap();
        assert!(validate_public_base_url(&url, AppEnv::Development).is_ok());
    }

    #[test]
    fn production_rejects_plaintext_localhost() {
        // The localhost convenience is for development/test only; production must
        // never accept plaintext, not even to the loopback address.
        let url = Url::parse("http://localhost:8000").unwrap();
        assert!(validate_public_base_url(&url, AppEnv::Production).is_err());
    }

    #[test]
    fn a_base_url_with_userinfo_is_invalid() {
        for raw in [
            "https://user:hunter2@fau.example",
            "https://user@fau.example",
        ] {
            let url = Url::parse(raw).unwrap();
            let e = validate_public_base_url(&url, AppEnv::Production).unwrap_err();
            assert_eq!(e.problem, ConfigProblem::Invalid, "{raw}");
            assert!(!e.to_string().contains("hunter2"));
        }
    }

    #[test]
    fn postgres_url_accepts_both_schemes() {
        for raw in ["postgres://u:p@db:5432/fau", "postgresql://u:p@db/fau"] {
            assert!(
                postgres_url("DATABASE_URL", raw.to_owned()).is_ok(),
                "{raw}"
            );
        }
    }

    #[test]
    fn postgres_url_rejects_malformed_values_by_name_only() {
        for raw in [
            "banana-sentinel",
            "http://u:banana-sentinel@db/fau",
            "postgres://u:banana-sentinel@db:notaport/fau",
        ] {
            let e = postgres_url("DATABASE_URL", raw.to_owned()).unwrap_err();
            assert_eq!(
                e.to_string(),
                "configuration variable DATABASE_URL is not a valid value"
            );
        }
    }

    #[test]
    fn development_rejects_a_non_loopback_plaintext_url() {
        let url = Url::parse("http://example.org").unwrap();
        assert!(validate_public_base_url(&url, AppEnv::Development).is_err());
    }
}
