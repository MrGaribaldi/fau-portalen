//! `fau register sync [--dry-run] [--seed]` (docs/school-register-design.md §5.1-5.3, and
//! the part 3 plan's "Handover to part 4").
//!
//! One connection carries the whole run, because it holds the session advisory lock. The
//! first snapshot only decides what to fetch. The plan is made from a second snapshot, read
//! in the REPEATABLE READ transaction that then applies it, so the register cannot change
//! between planning and applying.

use std::sync::Arc;

use fau_domain::register::source::NsrUnit;
use fau_domain::register::sync::{plan, RunKind, SyncInputs, SyncOutcome, SyncPlan};
use fau_domain::time::Moment;
use fau_persistence::register::{
    abort_reason_text, apply_plan, load_snapshot, record_aborted, record_applied, record_dry_run,
    record_failed, record_no_change, register_is_empty, seed_date, stage_payloads, start_run,
    try_lock, AppliedCounts, NsrPayload, RegisterError,
};
use fau_register_sources::client::SourceClient;
use fau_register_sources::SourceError;
use sqlx::{Connection, PgConnection};
use uuid::Uuid;

use super::fetch::{fetch, source_urls, FetchError};
use super::render::plan_json;
use super::Exit;
use crate::config::RegisterConfig;

/// Why a run failed. `Display` is fixed text plus a source's or the database's fixed
/// classification: never a URL, a body, a payload or an address. [`SyncError::code`] is the
/// even more restricted word actually persisted on the run row (controller hand-off): never
/// this `Display`, which -- though currently safe -- is not the contract a stored column
/// should depend on staying safe forever.
#[derive(Debug, thiserror::Error)]
enum SyncError {
    #[error("{0}")]
    Database(#[from] RegisterError),
    #[error("source error ({0})")]
    Source(#[from] SourceError),
    /// The apply transaction itself failed (`apply_plan`, `stage_payloads` or
    /// `record_applied`), after some of its statements may already have run. The caller must
    /// roll the transaction back explicitly and await that rollback before reusing the
    /// connection -- never rely on `Transaction`'s drop glue, which finishes on its own
    /// schedule and can race a query issued right after (controller hand-off).
    #[error("apply error ({0})")]
    Apply(RegisterError),
    /// One of the detail-fetch pool's workers panicked or was cancelled instead of returning
    /// a [`SourceError`] -- see [`FetchError`]. Never the panic payload: whatever it was, it
    /// stays inside the worker task tokio already isolated it to.
    #[error("an internal detail-fetch worker panicked")]
    WorkerPanicked,
    #[error("the register has no applied seed run to start the SSB lookback from")]
    NoSeed,
}

impl SyncError {
    /// `register_sync_runs.abort_reason` for a failed run: a fixed word, never this error's
    /// `Display`, a URL, a response body or an address.
    fn code(&self) -> &'static str {
        match self {
            SyncError::Database(_) => "database_error",
            SyncError::Source(_) => "source_error",
            SyncError::Apply(_) => "apply_error",
            SyncError::WorkerPanicked => "worker_panicked",
            SyncError::NoSeed => "no_seed",
        }
    }
}

impl From<sqlx::Error> for SyncError {
    fn from(e: sqlx::Error) -> Self {
        SyncError::Database(e.into())
    }
}

impl From<FetchError> for SyncError {
    fn from(e: FetchError) -> Self {
        match e {
            FetchError::Source(e) => SyncError::Source(e),
            FetchError::WorkerPanicked => SyncError::WorkerPanicked,
        }
    }
}

pub(super) async fn sync(config: &RegisterConfig, dry_run: bool, seed: bool) -> Exit {
    let at = Moment::at(jiff::Timestamp::now());
    let kind = if seed { RunKind::Seed } else { RunKind::Sync };
    // A dedicated connection, never a pooled one (controller hand-off): it holds the session
    // advisory lock for the whole run, and closing it -- on every exit path, including an
    // error or a panic -- is what releases that lock.
    let mut conn = match PgConnection::connect(config.database_url.expose()).await {
        Ok(conn) => conn,
        Err(e) => {
            let e = SyncError::from(e);
            tracing::error!(error = %e, "could not connect to the register database");
            return Exit::Failed;
        }
    };
    match begin(&mut conn, kind, dry_run, at).await {
        Ok(Begun::Run(run)) => match execute(&mut conn, config, kind, dry_run, run, at).await {
            Ok(exit) => exit,
            Err(e) => {
                tracing::error!(run_id = %run, error = %e, "register sync failed");
                // `conn` -- and the lock it holds -- stays open, right here, for the rest of
                // this call: only the write below uses a fresh connection (controller
                // hand-off). If the apply failed because `conn` itself broke (an io error, or
                // its backend killed, e.g. by a Postgres restart), reusing it to record the
                // failure would fail the same way and leave the run row unfinished forever.
                record_run_failure(config, run, e.code(), at).await;
                Exit::Failed
            }
        },
        Ok(Begun::Refused(exit)) => exit,
        Err(e) => {
            tracing::error!(error = %e, "register sync failed before it started");
            Exit::Failed
        }
    }
}

/// Records `run` as `failed`, on a brand-new connection -- never the one the failed run held
/// (controller hand-off, see [`sync`]). If even this fresh connection cannot be made or
/// cannot write, a fixed-text error goes to stderr and the caller still exits 1; the run row
/// is then left unfinished for real, which is what #3442's alerting on a stuck run watches
/// for.
async fn record_run_failure(config: &RegisterConfig, run: Uuid, reason: &'static str, at: Moment) {
    let mut fresh = match PgConnection::connect(config.database_url.expose()).await {
        Ok(fresh) => fresh,
        Err(_) => {
            tracing::error!(run_id = %run, "could not connect to record the run as failed");
            return;
        }
    };
    if record_failed(&mut fresh, run, reason, at).await.is_err() {
        tracing::error!(run_id = %run, "could not record the run as failed");
    }
}

enum Begun {
    Run(Uuid),
    Refused(Exit),
}

/// The lock, the empty-register rule (the Handover: `--seed` refuses a non-empty register,
/// and a sync refuses an empty one), then the run row. A refusal writes nothing.
async fn begin(
    conn: &mut PgConnection,
    kind: RunKind,
    dry_run: bool,
    at: Moment,
) -> Result<Begun, SyncError> {
    if !try_lock(conn).await? {
        tracing::error!("another run holds the lock");
        return Ok(Begun::Refused(Exit::Locked));
    }
    match (kind, register_is_empty(conn).await?) {
        (RunKind::Seed, false) => {
            tracing::error!("--seed refuses a register that is not empty");
            Ok(Begun::Refused(Exit::Refused))
        }
        (RunKind::Sync, true) => {
            tracing::error!("the register is empty: seed it first with --seed");
            Ok(Begun::Refused(Exit::Refused))
        }
        _ => {
            let run = start_run(conn, kind, dry_run, at).await?;
            tracing::info!(run_id = %run, kind = ?kind, dry_run, "register sync started");
            Ok(Begun::Run(run))
        }
    }
}

async fn execute(
    conn: &mut PgConnection,
    config: &RegisterConfig,
    kind: RunKind,
    dry_run: bool,
    run: Uuid,
    at: Moment,
) -> Result<Exit, SyncError> {
    let ssb_from = match kind {
        RunKind::Seed => None,
        RunKind::Sync => Some(seed_date(conn).await?.ok_or(SyncError::NoSeed)?),
    };
    let held = load_snapshot(conn).await?;
    let client = Arc::new(SourceClient::new(source_urls(config))?);
    let fetched = fetch(client, &held, kind, ssb_from, at.today()).await?;
    if fetched.absent_from_list > 0 {
        tracing::warn!(
            run_id = %run,
            count = fetched.absent_from_list,
            "register orgnrs absent from the NSR list are left untouched"
        );
    }
    let units: Vec<_> = fetched.units.iter().map(|(u, _)| u.clone()).collect();
    let inputs = SyncInputs {
        municipalities: &fetched.municipalities,
        code_changes: &fetched.code_changes,
        units: &units,
        kind,
        at,
    };

    let mut tx = conn.begin().await?;
    sqlx::query("set transaction isolation level repeatable read")
        .execute(&mut *tx)
        .await?;
    let snapshot = load_snapshot(&mut tx).await?;
    let outcome = plan(&snapshot, &inputs);

    if dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan_json(kind, &outcome, &snapshot))
                .expect("a JSON value always serialises")
        );
        tx.rollback().await?;
        return Ok(match &outcome {
            SyncOutcome::Abort { reason, counts } => {
                record_dry_run(conn, run, counts, Some(reason), at).await?;
                Exit::Aborted
            }
            SyncOutcome::Apply(plan) => {
                record_dry_run(conn, run, &plan.counts, None, at).await?;
                Exit::Done
            }
            SyncOutcome::NoChange => {
                record_dry_run(conn, run, &Default::default(), None, at).await?;
                Exit::Done
            }
        });
    }

    match outcome {
        SyncOutcome::NoChange => {
            tx.rollback().await?;
            record_no_change(conn, run, at).await?;
            tracing::info!(run_id = %run, outcome = "no_change", "register sync finished");
            Ok(Exit::Done)
        }
        SyncOutcome::Abort { reason, counts } => {
            tx.rollback().await?;
            // `record_aborted` writes the run row and its mail as two statements: wrapped in
            // its own transaction here (controller hand-off) so they commit together, never
            // one without the other. Rolled back explicitly on its own failure too, awaited
            // before `conn` is next used -- the same reasoning as `apply_and_stage`'s error
            // arm below, never `Transaction`'s drop glue.
            let mut record_tx = conn.begin().await?;
            if let Err(e) = record_aborted(&mut record_tx, run, kind, &reason, &counts, at).await {
                let _ = record_tx.rollback().await;
                return Err(SyncError::Database(e));
            }
            record_tx.commit().await?;
            tracing::error!(
                run_id = %run,
                abort_reason = %abort_reason_text(&reason),
                "register sync aborted"
            );
            Ok(Exit::Aborted)
        }
        SyncOutcome::Apply(plan) => {
            match apply_and_stage(&mut tx, &plan, fetched.units, run, kind, at).await {
                Ok(applied) if applied.wrote_nothing() => {
                    // Every review deduplicated away and there was no op: nothing to keep.
                    tx.rollback().await?;
                    record_no_change(conn, run, at).await?;
                    tracing::info!(
                        run_id = %run,
                        outcome = "no_change",
                        reviews_deduplicated = applied.deduplicated_reviews,
                        "register sync finished"
                    );
                    Ok(Exit::Done)
                }
                Ok(applied) => {
                    tx.commit().await?;
                    tracing::info!(
                        run_id = %run,
                        outcome = "applied",
                        ops = applied.ops,
                        reviews_written = applied.new_reviews.len(),
                        reviews_deduplicated = applied.deduplicated_reviews,
                        "register sync finished"
                    );
                    Ok(Exit::Done)
                }
                Err(e) => {
                    // The apply transaction failed, possibly after some of its statements
                    // already ran: roll it back explicitly and await that rollback -- never
                    // rely on `Transaction`'s drop glue -- before `conn` is touched again (its
                    // isolation-level setting, `begin`/`rollback` etc. are all on `conn`
                    // itself, not the dead `tx`). The failure itself is then recorded on yet
                    // another, fresh connection by `sync`'s caller (controller hand-off).
                    let _ = tx.rollback().await;
                    Err(SyncError::Apply(e))
                }
            }
        }
    }
}

/// Applies `plan`, stages its NSR payloads and records the applied run, all inside `tx`. Left
/// uncommitted either way: the caller commits on success, or rolls back on error -- explicitly,
/// never via `Transaction`'s drop glue (controller hand-off, see [`SyncError::Apply`]).
async fn apply_and_stage(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &SyncPlan<Uuid>,
    units: Vec<(NsrUnit, Vec<u8>)>,
    run: Uuid,
    kind: RunKind,
    at: Moment,
) -> Result<AppliedCounts, RegisterError> {
    let applied = apply_plan(tx, plan, at).await?;
    if applied.wrote_nothing() {
        return Ok(applied);
    }
    let payloads: Vec<NsrPayload> = units
        .into_iter()
        .map(|(unit, body)| NsrPayload::new(&unit, body))
        .collect();
    stage_payloads(tx, &payloads, at).await?;
    record_applied(tx, run, kind, &applied, at).await?;
    Ok(applied)
}
