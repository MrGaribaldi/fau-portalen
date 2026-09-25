//! The school register's persistence (#3441 part 4, docs/school-register-design.md §4, §5):
//! the snapshot the sync planner reads, the applier that writes its plan in one transaction,
//! and the run bookkeeping around it. Everything here runs as `fau_register` (D9); nothing
//! reads the database clock for a rule, so every function takes the caller's `Moment`.

mod apply;
mod error;
mod export;
mod reviews;
mod runs;
mod school_ops;
mod snapshot;
mod sql;
mod staging;

pub use apply::{apply_plan, AppliedCounts};
pub use error::RegisterError;
pub use export::{export_rows, ExportRow};
pub use reviews::NewReview;
pub use runs::{
    abort_reason_text, counts_json, record_aborted, record_applied, record_dry_run, record_failed,
    record_no_change, seed_date, start_run, try_lock, unlock, REGISTER_SYNC_LOCK_ID,
    SYNC_APPLIED_ACTION,
};
pub use snapshot::{load_snapshot, register_is_empty};
pub use staging::{stage_payloads, NsrPayload};
