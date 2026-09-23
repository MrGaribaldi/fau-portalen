//! The schema contract (design section 5): whether this binary is allowed to serve
//! the database it is pointed at. `schema_contract` answers a different question
//! than `_sqlx_migrations` -- not "which migration files have run" but "what
//! contract version does the database currently claim" -- so the application can
//! tolerate a database *ahead* of it, which is what lets an additive migration roll
//! out before the new pods do (ADR-001 expand/contract). Reading the version is
//! `fau-persistence`'s job; acting on the result -- refusing to serve, or treating
//! the database as merely unreachable -- is `fau serve`'s startup gate.

/// The lowest `schema_contract` version this binary can serve. Raised only when a
/// migration removes or changes something older code relies on -- never bumped just
/// because a migration added something additive, since [`is_compatible`] already
/// tolerates a database ahead of this number.
pub const MINIMUM_CONTRACT_VERSION: i32 = 2;

/// Whether `found` is safe for this binary to serve against: at or above its own
/// minimum. A database *above* the minimum is compatible on purpose -- see the
/// module doc comment on why the application must tolerate that.
pub fn is_compatible(found: i32) -> bool {
    found >= MINIMUM_CONTRACT_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lower_contract_is_incompatible() {
        assert!(!is_compatible(MINIMUM_CONTRACT_VERSION - 1));
    }

    #[test]
    fn an_equal_contract_is_compatible() {
        assert!(is_compatible(MINIMUM_CONTRACT_VERSION));
    }

    #[test]
    fn a_higher_contract_is_compatible() {
        // ADR-001 expand/contract: an additive migration rolls out before the new
        // pods do, so a running binary must tolerate a database ahead of it.
        assert!(is_compatible(MINIMUM_CONTRACT_VERSION + 1));
    }
}
