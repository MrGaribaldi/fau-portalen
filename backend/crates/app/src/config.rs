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

#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub app_env: AppEnv,
    pub http_bind: SocketAddr,
    pub public_base_url: Url,
    pub database_url: Secret<String>,
    pub db_pool_max_connections: u32,
    pub log_level: String,
    /// Telemetry and mail variables that exist in ADR-001's table but have no
    /// adapter yet. Present so a value never appears in this struct without
    /// reason, but per spec §4 they are recognised and accepted, not validated
    /// or used until then -- so this only records what was set, never rejects it.
    pub accepted_but_unused: Vec<(&'static str, String)>,
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

        let database_url = Secret::new(required("DATABASE_URL")?);

        let db_pool_max_connections: u32 = parsed(
            "DB_POOL_MAX_CONNECTIONS",
            &required("DB_POOL_MAX_CONNECTIONS")?,
        )?;
        if db_pool_max_connections == 0 {
            return Err(err("DB_POOL_MAX_CONNECTIONS", ConfigProblem::Invalid));
        }

        let log_level = optional("LOG_LEVEL").unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned());

        // Read but never validated, never used -- see ACCEPTED_BUT_UNUSED's doc
        // comment. A missing or garbage value here must not fail startup.
        let accepted_but_unused: Vec<(&'static str, String)> = ACCEPTED_BUT_UNUSED
            .iter()
            .filter_map(|&name| optional(name).map(|value| (name, value)))
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
    if host.is_empty() {
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
    /// Bounds the DDL *inside* a migration -- a table lock held by a long-running
    /// query. It does not bound the advisory lock; see `persistence::migrate`.
    pub lock_timeout_ms: u64,
    /// How long to wait for the outer advisory lock before giving up.
    pub lock_wait_ms: u64,
}

const DEFAULT_LOCK_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_LOCK_WAIT_MS: u64 = 30_000;

impl MigrateConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let migration_database_url = Secret::new(required("MIGRATION_DATABASE_URL")?);

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
    fn development_rejects_a_non_loopback_plaintext_url() {
        let url = Url::parse("http://example.org").unwrap();
        assert!(validate_public_base_url(&url, AppEnv::Development).is_err());
    }
}
