//! `fau register ...` (#3441, docs/school-register-design.md §5.1, §9): the weekly sync, its
//! seed, and the export. It runs as `fau_register` (D9), logs JSON to stderr, and keeps
//! stdout for its own output.
//!
//! Exit codes, which the CronJob (#3424) and an operator read:
//! - 0: the run finished: applied, no change, or a dry run printed;
//! - 1: it failed: configuration, the database, or a source;
//! - 2: bad arguments (clap's own);
//! - 3: the planner aborted it (an empty source or the circuit breaker, §5.3);
//! - 4: another run holds the lock;
//! - 5: refused: `--seed` on a register that is not empty, or a sync on an empty one.

mod export;
mod fetch;
mod render;
mod sync;

use std::process::ExitCode;

use crate::config::RegisterConfig;
use crate::telemetry::{self, LogStream};

#[derive(clap::Subcommand)]
pub(crate) enum RegisterCommand {
    /// Sync the register from NSR, Kartverket and SSB (§5.2).
    Sync {
        /// Plan, print the plan as JSON on stdout, and write nothing but a `dry_run` run row.
        #[arg(long)]
        dry_run: bool,
        /// The first run, on an empty register: creates every municipality and school.
        #[arg(long)]
        seed: bool,
        /// Apply a plan the mass-change circuit breaker stopped (§5.3), once its dry run has
        /// been read. Logged, and recorded in the run's audit entry. Never on a seed, which
        /// has no breaker.
        #[arg(long, conflicts_with = "seed")]
        accept_mass_change: bool,
    },
    /// Print every pickable school as CSV on stdout, for the prospect register (#3431).
    Export,
}

/// How a register command ended. The numbers are the process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    Done = 0,
    Failed = 1,
    Aborted = 3,
    Locked = 4,
    Refused = 5,
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        ExitCode::from(exit as u8)
    }
}

pub(crate) fn run(command: RegisterCommand, service_version: &'static str) -> ExitCode {
    let config = match RegisterConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            // Before any subscriber exists: plain text, the variable name and never its value.
            eprintln!("fau: {e}");
            return Exit::Failed.into();
        }
    };
    telemetry::init_to(&config.log_level, service_version, LogStream::Stderr);
    let rt = tokio::runtime::Runtime::new().expect("build a tokio runtime for `register`");
    let exit = rt.block_on(async {
        match command {
            RegisterCommand::Sync {
                dry_run,
                seed,
                accept_mass_change,
            } => sync::sync(&config, dry_run, seed, accept_mass_change).await,
            RegisterCommand::Export => export::export(&config).await,
        }
    });
    exit.into()
}
