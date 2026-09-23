//! The `fau` binary. One binary, one image, one digest across test, migration and
//! deploy -- which is what makes ADR-001's release barrier meaningful.

use std::process::ExitCode;

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

async fn run_server(config: ServeConfig) -> Result<(), StartupError> {
    // Lazy: an unreachable database must not be a startup failure (design section
    // 4). Nothing in this task uses the pool yet -- Task 9's readiness check is the
    // first caller -- but it is built here, up front, so startup order does not
    // change when that lands.
    let _pool =
        fau_persistence::lazy_pool(config.database_url.expose(), config.db_pool_max_connections)
            .map_err(StartupError::Pool)?;

    let state = http::AppState {
        readiness: readiness::ReadinessState::new(),
        version: SERVICE_VERSION,
    };

    // A plain startup line, not the JSON logging Task 10 adds -- just enough that
    // an operator watching a fresh container can see it got past configuration and
    // what it decided to do, without ever printing a secret. `database_url` is
    // deliberately absent: it is the one field here that carries a password. Reads
    // `state.version` rather than `SERVICE_VERSION` directly so the field on
    // `AppState` has a real use beyond being stored.
    eprintln!(
        "fau: starting {} ({:?} env) on {}, public base {}, log level {}",
        state.version, config.app_env, config.http_bind, config.public_base_url, config.log_level,
    );
    for (name, _value) in &config.accepted_but_unused {
        eprintln!("fau: accepted but not yet used: {name}");
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
