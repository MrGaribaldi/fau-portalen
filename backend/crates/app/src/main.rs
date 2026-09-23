//! The `fau` binary. One binary, one image, one digest across test, migration and
//! deploy -- which is what makes ADR-001's release barrier meaningful.

use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;

mod config;
mod http;
mod readiness;

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
        None => {
            eprintln!("fau: no command given; expected `serve` or `migrate`");
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

    // A synchronous top-level `main` (see `migrate`'s comment above) keeps
    // `--version` free of a runtime; `serve` builds its own here instead.
    let rt = tokio::runtime::Runtime::new().expect("build a tokio runtime for `serve`");
    rt.block_on(run_server(config))
}

/// Bounds the startup schema-contract query (ruling 6) so a black-holed database
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
/// path as an explicit low version, per ruling 2. Any other failure is classified by
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
        version: SERVICE_VERSION,
    };

    // A plain startup line, not the JSON logging Task 10 adds -- just enough that
    // an operator watching a fresh container can see it got past configuration and
    // what it decided to do, without ever printing a secret. `database_url` is
    // deliberately absent: it is the one field here that carries a password. Reads
    // `state.version` rather than `SERVICE_VERSION` directly so the field on
    // `AppState` has a real use beyond being stored. Printed before the
    // schema-contract check below, which does real network I/O and must not delay
    // this line -- an operator watching startup should see it got past
    // configuration immediately, whatever the gate decides next.
    eprintln!(
        "fau: starting {} ({:?} env) on {}, public base {}, log level {}",
        state.version, config.app_env, config.http_bind, config.public_base_url, config.log_level,
    );
    for (name, _value) in &config.accepted_but_unused {
        eprintln!("fau: accepted but not yet used: {name}");
    }

    // The gate runs once, here, right after the pool is built (design section 5),
    // before the router is built or anything is bound. It never performs DDL and
    // never repeats the DSN in any message it produces.
    match check_schema_contract(&pool).await {
        ContractCheck::Version(found) if !fau_domain::is_compatible(found) => {
            return Err(StartupError::SchemaContractBelowMinimum {
                found,
                minimum: fau_domain::MINIMUM_CONTRACT_VERSION,
            });
        }
        ContractCheck::Version(_) => {
            state.readiness.set_initialised();
        }
        ContractCheck::Retry(kind) => {
            // Plain stderr until Task 10's JSON logging. `kind` is a fixed, safe
            // description (never the sqlx `Display`, never a DSN). Local
            // initialisation is otherwise complete at this point, so readiness is
            // marked initialised here too -- it then stays false only because
            // `ReadinessState::probe`'s own database/contract check keeps failing,
            // and recovers on its own the moment the database answers again, with
            // no separate background reconnect loop.
            eprintln!(
                "fau: warning: could not verify the schema contract at startup \
                 ({kind}); continuing, readiness will retry"
            );
            state.readiness.set_initialised();
        }
        ContractCheck::Refuse(kind) => {
            return Err(StartupError::SchemaContractCheckFailed { kind });
        }
    }

    let router = http::router(state);

    let listener = tokio::net::TcpListener::bind(config.http_bind)
        .await
        .map_err(|e| StartupError::Bind {
            addr: config.http_bind,
            kind: e.kind(),
        })?;

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| StartupError::Serve(e.kind()))?;

    Ok(())
}

/// Minimal shutdown trigger, just enough for a test to end the process: ctrl-c or
/// SIGTERM. The 25-second in-flight budget, setting readiness false first and
/// structured shutdown logging are Task 11's work -- this only makes `serve` exit
/// instead of running forever.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install a SIGTERM handler");
        sig.recv().await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
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
