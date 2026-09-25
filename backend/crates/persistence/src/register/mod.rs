//! The school register's persistence (#3441 part 4, docs/school-register-design.md §4, §5):
//! the snapshot the sync planner reads, the applier that writes its plan in one transaction,
//! and the run bookkeeping around it. Everything here runs as `fau_register` (D9); nothing
//! reads the database clock for a rule, so every function takes the caller's `Moment`.

mod apply;
mod error;
mod reviews;
mod school_ops;
mod snapshot;
mod sql;
mod staging;

pub use apply::{apply_plan, AppliedCounts};
pub use error::RegisterError;
pub use reviews::NewReview;
pub use snapshot::{load_snapshot, register_is_empty};
pub use staging::{stage_payloads, NsrPayload};
