//! Readiness state for `/health/ready` (design section 9).
//!
//! This is a stub: it always reports ready, with no database check, no schema
//! contract check and no caching. Task 9 replaces the body with the real check --
//! local initialisation complete, a database ping bounded to one second, and the
//! schema contract at or above the binary's minimum -- but the type stays the same
//! shape so `http::health::ready` does not change when that lands.
#[derive(Debug, Clone)]
pub struct ReadinessState;

impl ReadinessState {
    pub fn new() -> Self {
        Self
    }

    /// Always `true` for now; see the module doc comment.
    pub async fn is_ready(&self) -> bool {
        true
    }
}

impl Default for ReadinessState {
    fn default() -> Self {
        Self::new()
    }
}
