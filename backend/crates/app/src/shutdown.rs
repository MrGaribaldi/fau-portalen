//! Graceful shutdown (design section 12).
//!
//! [`signal`] is the future `main.rs` hands to
//! `axum::serve(...).with_graceful_shutdown(...)`. It resolves on SIGTERM or SIGINT
//! (ctrl-c): first it sets `readiness` to report `ShuttingDown`
//! (`crate::readiness::ReadinessState::begin_shutdown`) and logs one JSON `INFO`
//! line, then it waits a short *pre-drain* before returning, so a `/health/ready`
//! scrape already in flight -- or one landing moments later, before the listener
//! actually stops accepting -- reliably observes 503 rather than racing the
//! listener closing. Only once this future resolves does axum stop accepting new
//! connections and start waiting for in-flight ones to finish.
//!
//! That "waiting for in-flight ones to finish" step has no bound of its own inside
//! axum -- left alone, a single request that never finishes (a stuck handler, an
//! open transaction, ...) would hang shutdown forever. [`signal`] arms a watchdog,
//! timed from the instant the signal was actually received (not from process
//! start, and counting the pre-drain sleep as part of it, not on top of it): if the
//! whole drain is not finished within [`DRAIN_BOUND`], the watchdog force-exits the
//! process itself with [`std::process::exit`], from inside its own spawned task,
//! independent of whatever `main.rs`'s `run_server` is still waiting on. Design
//! section 12: "an aborted transaction is never acknowledged to the client" is true
//! of this path because the process exit drops every remaining connection at once
//! -- including any transaction still open on it -- and PostgreSQL rolls back a
//! transaction whose connection disappears without a `commit`, before the response
//! that would have acknowledged it can ever be sent.
//!
//! [`signal`] only ever waits for the *first* SIGTERM or SIGINT: `wait_for_signal`'s
//! `tokio::signal` listeners fire once and are then dropped along with the rest of
//! this future, and nothing here creates new ones. A second SIGTERM or SIGINT
//! delivered while a drain is already under way is therefore not acted on again --
//! it does not restart the pre-drain, and it does not shorten or re-arm the
//! watchdog. The backstop for a process that still refuses to exit is
//! [`DRAIN_BOUND`] itself, or, past that, whatever the orchestrator does next (a
//! kubelet's default hard `SIGKILL` once its own termination grace period elapses)
//! -- never a second signal from the caller.

use std::time::Duration;

use crate::readiness::ReadinessState;

/// The total bound on the drain, from the moment the shutdown signal is received to
/// the point the process exits regardless of whether every in-flight request has
/// finished -- *including* [`PRE_DRAIN`], not on top of it (design section 12).
pub const DRAIN_BOUND: Duration = Duration::from_secs(25);

/// The exit code [`spawn_watchdog`] uses when it has to force the process to exit.
/// Non-zero, so a drain that was cut short is distinguishable in an exit-code metric
/// from an ordinary clean shutdown ([`std::process::ExitCode::SUCCESS`], `0`), even
/// though nothing in this codebase currently reads it back. There is no dedicated
/// `main.rs::StartupError` variant for this path: by the time the watchdog fires,
/// going through that `Result`-returning path would mean first waiting for the very
/// future ([`axum::serve`]'s graceful shutdown) that is the reason the watchdog had
/// to act, which defeats the point of a hard bound.
const DRAIN_TIMEOUT_EXIT_CODE: i32 = 1;

/// How long [`signal`] waits, after flipping readiness, before returning and
/// letting axum stop accepting new connections. Long enough that a readiness scrape
/// already in flight -- or one landing moments later -- reliably observes 503;
/// short enough to leave nearly all of [`DRAIN_BOUND`] for draining real in-flight
/// work. See `tests/shutdown.rs::sigterm_sets_readiness_false_before_the_listener_closes`.
const PRE_DRAIN: Duration = Duration::from_millis(500);

/// Resolves on SIGTERM or SIGINT (ctrl-c). See the module doc comment for the full
/// sequence; used as `axum::serve(...).with_graceful_shutdown(shutdown::signal(readiness))`.
pub async fn signal(readiness: ReadinessState) {
    wait_for_signal().await;

    // The first of two shutdown INFO lines, logged before readiness flips: a probe
    // already mid-flight when the signal arrives still reflects the state it
    // observed a moment ago, and this line marks the instant that stops being true.
    tracing::info!("shutdown: signal received, no longer accepting new work");
    readiness.begin_shutdown();

    // Armed here, not in `main.rs`: this is the instant the 25-second bound starts
    // counting from, and the pre-drain sleep just below must count against it, not
    // run before it starts.
    spawn_watchdog();

    tokio::time::sleep(PRE_DRAIN).await;
}

/// Spawns the task that force-exits the process if the drain -- the [`PRE_DRAIN`]
/// sleep [`signal`] is about to run, plus however long axum then waits for
/// in-flight requests -- is not finished within [`DRAIN_BOUND`] of this call. A
/// clean, on-time shutdown instead exits the process through `main`'s ordinary
/// `ExitCode` path well before this fires; at that point the whole process --
/// watchdog task included -- is already gone, so there is deliberately no handle
/// here to cancel it explicitly.
fn spawn_watchdog() {
    tokio::spawn(async {
        tokio::time::sleep(DRAIN_BOUND).await;
        tracing::warn!(
            bound_secs = DRAIN_BOUND.as_secs(),
            "shutdown: drain bound elapsed before every in-flight request finished; exiting anyway"
        );
        std::process::exit(DRAIN_TIMEOUT_EXIT_CODE);
    });
}

/// Waits for SIGINT (Ctrl-C) or SIGTERM, whichever comes first.
async fn wait_for_signal() {
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

// No unit tests in this module: `DRAIN_BOUND` and `DRAIN_TIMEOUT_EXIT_CODE` are
// plain constants, and a test that only compares one against itself proves nothing.
// The real behaviour they gate is proved end to end by
// `tests/shutdown.rs`: `the_process_exits_within_the_drain_bound` for a clean drain
// finishing well inside the bound, and `an_aborted_transaction_is_never_acknowledged`
// for the watchdog actually firing at the bound, with the right exit code and log
// line, when a request cannot finish in time.
