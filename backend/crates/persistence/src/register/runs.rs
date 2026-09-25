//! Run bookkeeping (docs/school-register-design.md §4.3, §5.1-5.3): the advisory lock that
//! keeps a manual run and the CronJob apart, the `register_sync_runs` row that is the alert
//! source (#3442), one audit entry per applied run, and the operator's mail.
//!
//! Audit and mail are global: no tenant, actor `system`, and params that hold ids, codes and
//! counts only. Mail goes to the operational address (§5.3) through 0004's global templates.
//!
//! Only [`start_run`] takes the run's `Moment`, for `started_at`. Every function that finishes
//! a run takes `finished_at`, the wall-clock time the run actually finished, which is also the
//! audit entry's `occurred_at` and the mail's `created_at` (final review, item 1).

use fau_domain::membership::rules::EWB_OVERSIGHT_ADDRESS;
use fau_domain::register::sync::{AbortReason, Counts, RunKind};
use fau_domain::time::{oslo_today, Moment};
use jiff::civil::Date;
use jiff::Timestamp;
use serde_json::{json, Value};
use sqlx::PgConnection;
use uuid::Uuid;

use super::apply::AppliedCounts;
use super::error::RegisterError;
use super::sql::{from_micros, ts_param};

/// The session-level advisory lock every `fau register sync` holds for its whole run. Fixed,
/// and distinct from `MIGRATION_LOCK_ID`.
pub const REGISTER_SYNC_LOCK_ID: i64 = 0x4641_5530_0000_3441;

/// The one audit action of an applied run.
pub const SYNC_APPLIED_ACTION: &str = "register.sync_applied";

/// Takes [`REGISTER_SYNC_LOCK_ID`] for this session without waiting. `false` means another
/// run holds it. The lock lives until [`unlock`] or until the connection closes, so the
/// caller keeps this one connection for the whole run.
pub async fn try_lock(conn: &mut PgConnection) -> Result<bool, RegisterError> {
    Ok(sqlx::query_scalar("select pg_try_advisory_lock($1)")
        .bind(REGISTER_SYNC_LOCK_ID)
        .fetch_one(&mut *conn)
        .await?)
}

pub async fn unlock(conn: &mut PgConnection) -> Result<(), RegisterError> {
    sqlx::query("select pg_advisory_unlock($1)")
        .bind(REGISTER_SYNC_LOCK_ID)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

fn kind_code(kind: RunKind) -> &'static str {
    match kind {
        RunKind::Seed => "seed",
        RunKind::Sync => "sync",
    }
}

/// `register_sync_runs.counts`: every planner count by name.
pub fn counts_json(c: &Counts) -> Value {
    json!({
        "municipalities_created": c.municipalities_created,
        "municipalities_updated": c.municipalities_updated,
        "renumbered": c.renumbered,
        "renamed": c.renamed,
        "schools_created": c.schools_created,
        "schools_renamed": c.schools_renamed,
        "schools_updated": c.schools_updated,
        "attribute_losses": c.attribute_losses,
        "schools_moved": c.schools_moved,
        "schools_closed": c.schools_closed,
        "schools_held": c.schools_held,
        "reviews": c.reviews,
        "skipped": c.skipped,
    })
}

/// `register_sync_runs.abort_reason`: a code and its numbers, never a name.
pub fn abort_reason_text(reason: &AbortReason) -> String {
    match reason {
        AbortReason::EmptySource { source } => format!("empty_source:{source}"),
        AbortReason::MassChange {
            closes,
            renames,
            attribute_losses,
            active,
        } => format!(
            "mass_change:closes={closes},renames={renames},attribute_losses={attribute_losses},active={active}"
        ),
    }
}

/// Inserts the run's row, unfinished, and returns its id. Written before anything is
/// fetched, so a run that dies part-way leaves a row with no outcome behind.
pub async fn start_run(
    conn: &mut PgConnection,
    kind: RunKind,
    dry_run: bool,
    at: Moment,
) -> Result<Uuid, RegisterError> {
    let id = Uuid::now_v7();
    let code = if dry_run { "dry_run" } else { kind_code(kind) };
    sqlx::query(
        "insert into register_sync_runs (id, kind, started_at) values ($1, $2, $3::timestamptz)",
    )
    .bind(id)
    .bind(code)
    .bind(ts_param(at.now()))
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

async fn finish(
    conn: &mut PgConnection,
    run: Uuid,
    outcome: &str,
    counts: Value,
    abort_reason: Option<&str>,
    finished_at: Timestamp,
) -> Result<(), RegisterError> {
    let updated = sqlx::query(
        "update register_sync_runs
            set finished_at = $2::timestamptz, outcome = $3, counts = $4::jsonb,
                abort_reason = $5
          where id = $1 and finished_at is null",
    )
    .bind(run)
    .bind(ts_param(finished_at))
    .bind(outcome)
    .bind(counts.to_string())
    .bind(abort_reason)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RegisterError::UnknownRow)
    }
}

/// §5.3: nothing to do. Writes nothing but the run row.
pub async fn record_no_change(
    conn: &mut PgConnection,
    run: Uuid,
    finished_at: Timestamp,
) -> Result<(), RegisterError> {
    finish(
        conn,
        run,
        "no_change",
        counts_json(&Counts::default()),
        None,
        finished_at,
    )
    .await
}

/// A dry run changed nothing, so its outcome is `no_change`; its counts and abort reason say
/// what the real run would have done.
pub async fn record_dry_run(
    conn: &mut PgConnection,
    run: Uuid,
    counts: &Counts,
    abort: Option<&AbortReason>,
    finished_at: Timestamp,
) -> Result<(), RegisterError> {
    let reason = abort.map(abort_reason_text);
    finish(
        conn,
        run,
        "no_change",
        counts_json(counts),
        reason.as_deref(),
        finished_at,
    )
    .await
}

/// A run that failed before it could plan or apply, e.g. a source that did not answer.
/// `reason` is a fixed description, never a response body or a URL.
pub async fn record_failed(
    conn: &mut PgConnection,
    run: Uuid,
    reason: &str,
    finished_at: Timestamp,
) -> Result<(), RegisterError> {
    finish(conn, run, "failed", json!({}), Some(reason), finished_at).await
}

/// The circuit breaker or an empty source stopped the run (§5.3): the run row and the
/// `register.sync_aborted` mail, and nothing else.
pub async fn record_aborted(
    conn: &mut PgConnection,
    run: Uuid,
    kind: RunKind,
    reason: &AbortReason,
    counts: &Counts,
    finished_at: Timestamp,
) -> Result<(), RegisterError> {
    let text = abort_reason_text(reason);
    finish(
        conn,
        run,
        "aborted",
        counts_json(counts),
        Some(&text),
        finished_at,
    )
    .await?;
    let mut params = json!({ "run_id": run, "kind": kind_code(kind) });
    match reason {
        AbortReason::EmptySource { source } => {
            params["reason"] = json!("empty_source");
            params["source"] = json!(source);
        }
        AbortReason::MassChange {
            closes,
            renames,
            attribute_losses,
            active,
        } => {
            params["reason"] = json!("mass_change");
            params["closes"] = json!(closes);
            params["renames"] = json!(renames);
            params["attribute_losses"] = json!(attribute_losses);
            params["active"] = json!(active);
        }
    }
    enqueue(conn, "register.sync_aborted", params, finished_at).await
}

/// An applied run, inside the transaction that applied it: the run row, one audit entry, and
/// the mail. A seed sends one `register.seed_summary` with the counts rather than a mail per
/// review item (Erik, 24 September 2026); a sync sends one `register.review_item` per item it
/// wrote.
pub async fn record_applied(
    conn: &mut PgConnection,
    run: Uuid,
    kind: RunKind,
    applied: &AppliedCounts,
    finished_at: Timestamp,
) -> Result<(), RegisterError> {
    let mut counts = counts_json(&applied.counts);
    counts["reviews_written"] = json!(applied.new_reviews.len());
    counts["reviews_deduplicated"] = json!(applied.deduplicated_reviews);
    finish(conn, run, "applied", counts.clone(), None, finished_at).await?;

    let mut params = counts.clone();
    params["kind"] = json!(kind_code(kind));
    sqlx::query(
        "insert into audit_events
           (id, tenant_id, actor_kind, action, subject_type, subject_id, occurred_at, params)
         values ($1, null, 'system', $2, 'register_sync_run', $3, $4::timestamptz, $5::jsonb)",
    )
    .bind(Uuid::now_v7())
    .bind(SYNC_APPLIED_ACTION)
    .bind(run)
    .bind(ts_param(finished_at))
    .bind(params.to_string())
    .execute(&mut *conn)
    .await?;

    match kind {
        RunKind::Seed => {
            enqueue(
                conn,
                "register.seed_summary",
                json!({ "run_id": run, "counts": counts }),
                finished_at,
            )
            .await
        }
        RunKind::Sync => {
            for review in &applied.new_reviews {
                enqueue(
                    conn,
                    "register.review_item",
                    json!({
                        "run_id": run,
                        "review_item_id": review.id,
                        "kind": review.kind.code(),
                    }),
                    finished_at,
                )
                .await?;
            }
            Ok(())
        }
    }
}

async fn enqueue(
    conn: &mut PgConnection,
    template: &'static str,
    params: Value,
    created_at: Timestamp,
) -> Result<(), RegisterError> {
    sqlx::query(
        "insert into outbox (id, tenant_id, template, recipient_email, params, created_at)
         values ($1, null, $2, $3, $4::jsonb, $5::timestamptz)",
    )
    .bind(Uuid::now_v7())
    .bind(template)
    .bind(EWB_OVERSIGHT_ADDRESS)
    .bind(params.to_string())
    .bind(ts_param(created_at))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Where the fixed SSB lookback starts (the Handover): the Oslo date of the earliest applied
/// seed. `None` before the register is seeded.
pub async fn seed_date(conn: &mut PgConnection) -> Result<Option<Date>, RegisterError> {
    let started: Option<i64> = sqlx::query_scalar(
        "select (extract(epoch from min(started_at)) * 1000000)::bigint
           from register_sync_runs where kind = 'seed' and outcome = 'applied'",
    )
    .fetch_one(&mut *conn)
    .await?;
    started
        .map(|us| from_micros(us).map(oslo_today))
        .transpose()
}
