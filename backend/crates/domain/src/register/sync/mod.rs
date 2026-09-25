//! The register sync planner (#3441, docs/school-register-design.md §2.4, §4.5, §5.2-5.3).
//! Pure: it reads a snapshot of the register and the freshly fetched sources, and returns a
//! plan of what to create, rename, renumber, move and close, plus the review items. No SQL,
//! no HTTP, no mail: the persistence applier and `fau register sync` apply the plan.

// Wired into `plan()` in Task 5; until then only its tests call it.
#[allow(dead_code)]
mod municipalities;
// Wired into `plan()` in Task 5; until then only its tests call it.
#[allow(dead_code)]
mod schools;
mod similarity;
#[cfg(test)]
mod testkit;
mod types;

pub use similarity::{similarity, REREGISTRATION_THRESHOLD, SUBMISSION_MATCH_THRESHOLD};
pub use types::*;
