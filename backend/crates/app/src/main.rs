//! The `fau` binary. One binary, one image, one digest across test, migration and
//! deploy -- which is what makes ADR-001's release barrier meaningful.

use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;

mod config;
mod http;
mod readiness;
mod register;
mod shutdown;
mod telemetry;

use config::{MigrateConfig, ServeConfig};

/// Supplied by the image build. A local `cargo build` does not set it, and an
/// unknown revision is better than a wrong one.
const BUILD_REVISION: &str = match option_env!("FAU_BUILD_REVISION") {
    Some(rev) => rev,
    None => "unknown",
};

const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "fau", disable_version_flag = true)]
struct Cli {
    /// Print the build revision and exit. Reads no configuration. Global so it
    /// works after a subcommand too, e.g. `fau serve --version`.
    #[arg(long, global = true)]
    version: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Serve HTTP. Never performs DDL.
    Serve,
    /// Apply pending migrations and exit.
    Migrate,
    /// The school register (#3441): sync it from its public sources, or export it.
    Register {
        #[command(subcommand)]
        command: register::RegisterCommand,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Before anything else: --version must not read configuration or touch a secret.
    if cli.version {
        println!("fau {SERVICE_VERSION} ({BUILD_REVISION})");
        return ExitCode::SUCCESS;
    }

    match cli.command {
        Some(Command::Serve) => run(serve),
        Some(Command::Migrate) => run(migrate),
        Some(Command::Register { command }) => register::run(command, SERVICE_VERSION),
        None => {
            eprintln!("fau: no command given; expected `serve`, `migrate` or `register`");
            ExitCode::FAILURE
        }
    }
}

/// Configuration errors are reported before any logging subscriber is installed, so
/// they go to stderr as plain text. They carry a variable name and never a value.
fn run(f: impl FnOnce() -> Result<(), StartupError>) -> ExitCode {
    match f() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fau: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("{0}")]
    Config(#[from] config::ConfigError),
    #[error("{0}")]
    Migrate(#[from] fau_persistence::MigrateError),
    #[error("database pool configuration error: {0}")]
    Pool(fau_persistence::ConnectErrorKind),
    #[error(
        "schema contract: database is at version {found}, this binary requires at least {minimum}"
    )]
    SchemaContractBelowMinimum { found: i32, minimum: i32 },
    #[error("schema contract: could not verify it at startup ({kind})")]
    SchemaContractCheckFailed { kind: String },
    #[error("failed to bind {addr}: {kind:?}")]
    Bind {
        addr: std::net::SocketAddr,
        kind: std::io::ErrorKind,
    },
    #[error("http server error: {0:?}")]
    Serve(std::io::ErrorKind),
}

fn serve() -> Result<(), StartupError> {
    let config = ServeConfig::from_env()?;

    // Installed as early as possible after configuration succeeds: from this point
    // on, JSON to stdout is the only log transport (design section 10) -- no more
    // `eprintln!`, anywhere, on any path below this line.
    telemetry::init(&config.log_level, SERVICE_VERSION);

    // A synchronous top-level `main` (see `migrate`'s comment above) keeps
    // `--version` free of a runtime; `serve` builds its own here instead.
    let rt = tokio::runtime::Runtime::new().expect("build a tokio runtime for `serve`");
    rt.block_on(run_server(config))
}

/// Bounds the startup schema-contract query so a black-holed database
/// cannot delay startup indefinitely -- distinct from, and much tighter than, any
/// bound the pool itself applies once it is in regular use.
const SCHEMA_CONTRACT_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// What the startup schema-contract check found.
enum ContractCheck {
    /// The query completed and reported this version.
    Version(i32),
    /// The query did not complete, but the failure is plausibly transient --
    /// `fau_persistence::is_retryable` -- or this check's own bound elapsed. Design
    /// section 4: a merely unreachable database is not a contract failure, so this
    /// is not a refusal. Carries a safe, fixed description of what happened (never
    /// the sqlx `Display`) for the warning message.
    Retry(String),
    /// The query failed with something a retry will not fix (e.g. a privilege
    /// problem, or an unexpected decode failure). Carries the same kind of safe
    /// description as `Retry`, for the refusal message.
    Refuse(String),
}

/// A fixed description for the check's own bound elapsing -- never constructed from
/// any value the database or the caller supplied, so it is as safe to print as
/// `fau_persistence::safe_error_kind`'s fixed words.
const CONTRACT_CHECK_TIMED_OUT: &str = "timed out waiting for a response";

/// Runs the startup schema-contract check under [`SCHEMA_CONTRACT_CHECK_TIMEOUT`]. A
/// missing `schema_contract` table (SQLSTATE 42P01) means an unmigrated database and
/// is reported as version 0 -- below any real minimum, so it takes the same refusal
/// path as an explicit low version. Any other failure is classified by
/// `fau_persistence::is_retryable` into [`ContractCheck::Retry`] (the caller warns
/// and lets `serve` continue) or [`ContractCheck::Refuse`] (the caller exits
/// non-zero) -- see that function's doc comment for the exact SQLSTATE list. Reuses
/// `fau_persistence::safe_error_kind` for the message in both cases rather than a
/// second classification here.
async fn check_schema_contract(pool: &sqlx::PgPool) -> ContractCheck {
    match tokio::time::timeout(
        SCHEMA_CONTRACT_CHECK_TIMEOUT,
        fau_persistence::read_contract_version(pool),
    )
    .await
    {
        Ok(Ok(version)) => ContractCheck::Version(version),
        Ok(Err(e)) if fau_persistence::is_undefined_table(&e) => ContractCheck::Version(0),
        Ok(Err(e)) if fau_persistence::is_retryable(&e) => {
            ContractCheck::Retry(fau_persistence::safe_error_kind(&e))
        }
        Ok(Err(e)) => ContractCheck::Refuse(fau_persistence::safe_error_kind(&e)),
        Err(_elapsed) => ContractCheck::Retry(CONTRACT_CHECK_TIMED_OUT.to_owned()),
    }
}

async fn run_server(config: ServeConfig) -> Result<(), StartupError> {
    // Lazy: an unreachable database must not be a startup failure (design section
    // 4). `connect_lazy_with` never dials the database itself, so building the pool
    // here cannot fail on connectivity -- only the schema-contract check just below,
    // which is bounded and treats a connection failure as "unreachable", not a
    // startup error.
    let pool =
        fau_persistence::lazy_pool(config.database_url.expose(), config.db_pool_max_connections)
            .map_err(StartupError::Pool)?;

    let state = http::AppState {
        readiness: readiness::ReadinessState::new(),
        pool: pool.clone(),
    };

    // One JSON startup event, so log collection sees startup like any other line.
    // `database_url` is deliberately absent: it is the one field here that carries
    // a password.
    // `service_version` is not passed explicitly -- `telemetry::JsonLineLayer`
    // inserts it into every line unconditionally, this one included. Logged before
    // the schema-contract check below, which does real network I/O and must not
    // delay this event -- an operator (or a dashboard) watching startup should see
    // it got past configuration immediately, whatever the gate decides next.
    tracing::info!(
        app_env = %config.app_env,
        http_bind = %config.http_bind,
        public_base_url = %config.public_base_url,
        log_level = %config.log_level,
        "fau starting"
    );
    for name in &config.accepted_but_unused {
        tracing::warn!(variable = name, "accepted but not yet used");
    }

    // The gate runs once, here, right after the pool is built (design section 5),
    // before the router is built or anything is bound. It never performs DDL and
    // never repeats the DSN in any message it produces.
    match check_schema_contract(&pool).await {
        ContractCheck::Version(found) if !fau_domain::is_compatible(found) => {
            let err = StartupError::SchemaContractBelowMinimum {
                found,
                minimum: fau_domain::MINIMUM_CONTRACT_VERSION,
            };
            // A JSON ERROR event on stdout for log collection, in addition to (not instead
            // of) the final plain `fau: ...` line `run()` still prints to stderr.
            tracing::error!(error = %err, "refusing to serve");
            return Err(err);
        }
        ContractCheck::Version(_) => {
            state.readiness.set_initialised();
        }
        ContractCheck::Retry(kind) => {
            // A JSON WARN event on stdout only -- this path does not
            // refuse to start, so there is no final stderr exit line to pair it
            // with. `kind` is a fixed, safe description (never the sqlx `Display`,
            // never a DSN). Local initialisation is otherwise complete at this
            // point, so readiness is marked initialised here too -- it then stays
            // false only because `ReadinessState::probe`'s own database/contract
            // check keeps failing, and recovers on its own the moment the database
            // answers again, with no separate background reconnect loop.
            tracing::warn!(
                kind,
                "could not verify the schema contract at startup; continuing, readiness will retry"
            );
            state.readiness.set_initialised();
        }
        ContractCheck::Refuse(kind) => {
            let err = StartupError::SchemaContractCheckFailed { kind };
            tracing::error!(error = %err, "refusing to serve");
            return Err(err);
        }
    }

    // Cloned before `state` is moved into `http::router` below -- `shutdown::signal`
    // needs its own handle on the same readiness state so SIGTERM can flip it.
    let readiness_for_shutdown = state.readiness.clone();

    let router = http::router(state);

    let listener = tokio::net::TcpListener::bind(config.http_bind)
        .await
        .map_err(|e| StartupError::Bind {
            addr: config.http_bind,
            kind: e.kind(),
        });
    let listener = match listener {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!(error = %err, "failed to bind");
            return Err(err);
        }
    };

    match axum::serve(listener, router)
        .with_graceful_shutdown(shutdown::signal(readiness_for_shutdown))
        .await
    {
        Ok(()) => {
            // The second shutdown INFO line, pairing with `shutdown::signal`'s
            // "signal received" line. Only reached on a clean drain: the
            // drain-bound timeout path exits the process directly from
            // `shutdown::spawn_watchdog`, from a task independent of this `.await`,
            // so a timed-out drain never reaches this line at all.
            tracing::info!("shutdown: drain complete, exiting");
            Ok(())
        }
        Err(e) => {
            let err = StartupError::Serve(e.kind());
            tracing::error!(error = %err, "http server error");
            Err(err)
        }
    }
}

fn migrate() -> Result<(), StartupError> {
    let config = MigrateConfig::from_env()?;

    // `migrate` is a one-shot command, not a long-lived server, so a runtime built
    // here -- rather than `#[tokio::main]` on `main` -- keeps `--version` and a
    // future synchronous `serve` startup path free of one.
    let rt = tokio::runtime::Runtime::new().expect("build a tokio runtime for `migrate`");
    rt.block_on(async {
        let settings = fau_persistence::MigrationSettings {
            url: config.migration_database_url.expose().clone(),
            lock_timeout_ms: config.lock_timeout_ms,
            lock_wait_ms: config.lock_wait_ms,
        };
        fau_persistence::run_migrations(&settings).await
    })?;

    Ok(())
}
