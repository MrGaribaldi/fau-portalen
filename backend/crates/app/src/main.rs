//! The `fau` binary. One binary, one image, one digest across test, migration and
//! deploy -- which is what makes ADR-001's release barrier meaningful.

use std::process::ExitCode;

use clap::Parser;

mod config;

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
}

fn serve() -> Result<(), StartupError> {
    let _config = ServeConfig::from_env()?;
    todo!("serve")
}

fn migrate() -> Result<(), StartupError> {
    let _config = MigrateConfig::from_env()?;
    todo!("migrate")
}
