//! Readiness state for `/health/ready` (design section 9).
//!
//! Evaluated per scrape, not by a background reconnect loop: local initialisation
//! complete, then a database check (`fau_persistence::db_check`) and the schema
//! contract compared against this binary's minimum
//! (`fau_persistence::read_contract_version` and `fau_domain::is_compatible`), run
//! sequentially. **Both queries together** are bounded to one second by a single
//! outer [`tokio::time::timeout`] in [`check`] -- `db_check` carries its own
//! one-second timeout too, but `read_contract_version` does not, so without this
//! outer bound a lock held on `schema_contract` (e.g. by a migration) could hang a
//! scrape indefinitely even though the database itself answered instantly. A
//! successful probe is cached for 1000ms; a failing probe -- including one that hit
//! this outer timeout -- is never cached, so recovery is visible on the very next
//! scrape rather than delayed by up to a second.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tokio::sync::Mutex;

/// How long a `Ready` result is reused before the next probe re-checks the database
/// and schema contract. A `NotReady` result is never cached -- see the module doc
/// comment.
const CACHE_TTL: Duration = Duration::from_millis(1000);

/// The bound on the *whole* check -- `db_check` plus `read_contract_version`, run
/// sequentially -- not just the first query. Reuses `fau_persistence::DB_CHECK_TIMEOUT`
/// (one second, design section 9) rather than a second literal.
const READINESS_CHECK_TIMEOUT: Duration = fau_persistence::DB_CHECK_TIMEOUT;

/// Why `/health/ready` is not ready. Every field here is safe to put straight into
/// the HTTP response body (ADR-001: minimal, no internal addresses) -- `found` and
/// `minimum` are schema-contract version integers from our own migrations, never a
/// value the database's connection details could leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotReadyReason {
    /// `set_initialised` has not been called yet -- local startup is still running.
    Initialising,
    /// `fau_persistence::db_check` failed or timed out.
    Database,
    /// The database's schema contract is below this binary's minimum.
    SchemaContract { found: i32, minimum: i32 },
    /// `begin_shutdown` has been called; SIGTERM is in flight (Task 11).
    ShuttingDown,
}

/// The result of one readiness probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    NotReady(NotReadyReason),
}

/// A cached `Ready` result and when it was taken. Never holds a `NotReady` result --
/// see the module doc comment on why a failing probe is never cached.
struct CachedReady {
    at: Instant,
}

struct Inner {
    initialised: AtomicBool,
    shutting_down: AtomicBool,
    cache: Mutex<Option<CachedReady>>,
    /// Whether the most recent real database/schema-contract check (`check`, below)
    /// was `Ready`. Used only to rate-limit [`check`]'s own warning to a state
    /// transition (ready -> not ready), per ruling 7 -- not part of the readiness
    /// result itself, and never consulted by `probe_with`'s cache logic.
    was_ready: AtomicBool,
}

/// Shared readiness state (design section 9). `Clone` because axum's `State`
/// extractor requires it; cheap to clone -- an `Arc` around two atomics and a small
/// cache.
#[derive(Clone)]
pub struct ReadinessState(Arc<Inner>);

impl ReadinessState {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            initialised: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            cache: Mutex::new(None),
            was_ready: AtomicBool::new(true),
        }))
    }

    /// Marks local startup complete. Called once on `serve`'s success path, and
    /// also once on its "database unreachable at startup, keep running" path once
    /// local initialisation is otherwise done: readiness then stays false only
    /// because [`ReadinessState::probe`]'s own database/contract check fails, and
    /// recovers on its own the moment the database answers again -- there is no
    /// separate background reconnect loop to wire up.
    pub fn set_initialised(&self) {
        self.0.initialised.store(true, Ordering::SeqCst);
    }

    /// Makes readiness false immediately, bypassing the cache entirely -- Task 11
    /// wires SIGTERM to this. Every probe from this call onward reports
    /// [`NotReadyReason::ShuttingDown`], regardless of the database's state or
    /// whatever `Ready` result was cached a moment ago.
    ///
    /// Not called anywhere yet outside this module's own unit test: `main.rs`'s
    /// `shutdown_signal` stays exactly as Task 8 left it until Task 11 wires it up,
    /// per this task's own ruling. `#[allow(dead_code)]` documents that gap rather
    /// than hiding it.
    #[allow(dead_code)]
    pub fn begin_shutdown(&self) {
        self.0.shutting_down.store(true, Ordering::SeqCst);
    }

    /// Evaluates readiness against a real database: shutting down, then initialised,
    /// then a cached `Ready` result still inside [`CACHE_TTL`], then a fresh check
    /// bounded to about one second overall (see [`check`]). Only a fresh `Ready`
    /// result is cached; `NotReady` is returned as-is and never stored, so the very
    /// next call re-checks for real.
    pub async fn probe(&self, pool: &PgPool) -> Readiness {
        self.probe_with(|| check(pool, &self.0.was_ready)).await
    }

    /// The shared implementation behind [`ReadinessState::probe`], parameterised
    /// over the check itself so a unit test can substitute a fake, instant check
    /// function -- proving the cache's exact behaviour (a failure is never cached, a
    /// success is, for exactly [`CACHE_TTL`]) deterministically, without a live
    /// database or a timing-dependent integration test. See
    /// `tests::a_failure_is_never_cached_so_the_next_call_rechecks` and
    /// `tests::a_success_is_cached_for_the_ttl`.
    async fn probe_with<F, Fut>(&self, check: F) -> Readiness
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Readiness>,
    {
        if self.0.shutting_down.load(Ordering::SeqCst) {
            return Readiness::NotReady(NotReadyReason::ShuttingDown);
        }
        if !self.0.initialised.load(Ordering::SeqCst) {
            return Readiness::NotReady(NotReadyReason::Initialising);
        }

        {
            let cache = self.0.cache.lock().await;
            if let Some(cached) = cache.as_ref() {
                if cached.at.elapsed() < CACHE_TTL {
                    return Readiness::Ready;
                }
            }
        }

        let result = check().await;

        // The check just awaited may have taken a while (up to
        // `READINESS_CHECK_TIMEOUT`); re-read `shutting_down` rather than trusting
        // the read from before the check started, so a probe already in flight when
        // `begin_shutdown` is called cannot still report `Ready` -- or get its
        // result cached -- once shutdown is under way.
        if self.0.shutting_down.load(Ordering::SeqCst) {
            return Readiness::NotReady(NotReadyReason::ShuttingDown);
        }

        if matches!(result, Readiness::Ready) {
            *self.0.cache.lock().await = Some(CachedReady { at: Instant::now() });
        }

        result
    }
}

impl Default for ReadinessState {
    fn default() -> Self {
        Self::new()
    }
}

/// The actual database and schema-contract check, run fresh on every call that
/// reaches it -- caching is [`ReadinessState::probe`]'s concern, not this
/// function's. Bounded to [`READINESS_CHECK_TIMEOUT`] *as a whole*: `db_check` and
/// `read_contract_version` run sequentially inside the single outer
/// `tokio::time::timeout` below, so a lock held on `schema_contract` (which only the
/// second query would ever wait on -- `db_check`'s `select 1` never touches that
/// table) cannot hang a scrape past this bound. A timeout here is reported the same
/// as any other database failure: [`NotReadyReason::Database`], never a distinct
/// reason, since it says nothing about the schema contract's actual version.
async fn check(pool: &PgPool, was_ready: &AtomicBool) -> Readiness {
    match tokio::time::timeout(READINESS_CHECK_TIMEOUT, run_checks(pool, was_ready)).await {
        Ok(result) => result,
        Err(_elapsed) => {
            if should_warn(was_ready) {
                tracing::warn!(
                    reason = "database",
                    kind = "timed out waiting for a response",
                    "readiness probe failed"
                );
            }
            Readiness::NotReady(NotReadyReason::Database)
        }
    }
}

/// Whether a just-observed failure is worth a `WARN` line: true exactly once per
/// transition from ready into not-ready (ruling 7), so a probe polled every few
/// seconds during an extended outage produces one line, not one per scrape. Reset by
/// [`note_ready`] the next time a check succeeds, so the next failure after a
/// recovery logs again. The message itself stays at each call site so it can carry
/// whatever safe, structured fields are relevant there (a fixed `kind` string for a
/// database failure, `found`/`minimum` integers for a schema-contract one) rather
/// than being forced through one generic shape -- never the DSN or a database error's
/// own `Display` either way.
fn should_warn(was_ready: &AtomicBool) -> bool {
    was_ready.swap(false, Ordering::SeqCst)
}

/// The other half of [`should_warn`]'s state machine: marks the next failure as worth
/// logging again.
fn note_ready(was_ready: &AtomicBool) {
    was_ready.store(true, Ordering::SeqCst);
}

/// The two queries `check` bounds together, run sequentially with no timeout of
/// their own at this level (`db_check` has its own inner one-second bound as a
/// defence in depth; `read_contract_version` has none until `check`'s outer timeout
/// wraps it here).
async fn run_checks(pool: &PgPool, was_ready: &AtomicBool) -> Readiness {
    if let Err(e) = fau_persistence::db_check(pool).await {
        if should_warn(was_ready) {
            tracing::warn!(reason = "database", kind = %e, "readiness probe failed");
        }
        return Readiness::NotReady(NotReadyReason::Database);
    }

    match fau_persistence::read_contract_version(pool).await {
        Ok(found) if fau_domain::is_compatible(found) => {
            note_ready(was_ready);
            Readiness::Ready
        }
        Ok(found) => {
            if should_warn(was_ready) {
                tracing::warn!(
                    reason = "schema_contract",
                    found,
                    minimum = fau_domain::MINIMUM_CONTRACT_VERSION,
                    "readiness probe failed"
                );
            }
            Readiness::NotReady(NotReadyReason::SchemaContract {
                found,
                minimum: fau_domain::MINIMUM_CONTRACT_VERSION,
            })
        }
        // A database that has never been migrated reads as contract version 0 --
        // below any real minimum, so it takes the same not-ready path as an
        // explicit low version (mirrors `main.rs`'s startup gate).
        Err(e) if fau_persistence::is_undefined_table(&e) => {
            if should_warn(was_ready) {
                tracing::warn!(
                    reason = "schema_contract",
                    found = 0,
                    minimum = fau_domain::MINIMUM_CONTRACT_VERSION,
                    "readiness probe failed"
                );
            }
            Readiness::NotReady(NotReadyReason::SchemaContract {
                found: 0,
                minimum: fau_domain::MINIMUM_CONTRACT_VERSION,
            })
        }
        // Any other failure reading the contract (connection dropped between the
        // two queries, a privilege problem, ...) is reported as a database problem
        // rather than a schema-contract one -- `db_check` just above already proved
        // the database itself was reachable a moment earlier, so this is a
        // transient or narrower issue, not evidence the schema is wrong.
        Err(e) => {
            if should_warn(was_ready) {
                let kind = fau_persistence::safe_error_kind(&e);
                tracing::warn!(reason = "database", kind, "readiness probe failed");
            }
            Readiness::NotReady(NotReadyReason::Database)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool that never actually dials out: `begin_shutdown` and `Initialising`
    /// both short-circuit in `probe` before the database is ever touched, so a
    /// lazy pool pointed at nothing is safe to use here.
    fn untouched_pool() -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://u:p@127.0.0.1:1/none")
            .expect("build a lazy pool")
    }

    #[tokio::test]
    async fn not_ready_while_uninitialised() {
        let state = ReadinessState::new();
        assert_eq!(
            state.probe(&untouched_pool()).await,
            Readiness::NotReady(NotReadyReason::Initialising)
        );
    }

    #[tokio::test]
    async fn begin_shutdown_makes_readiness_false_immediately() {
        let state = ReadinessState::new();
        state.set_initialised();
        state.begin_shutdown();
        assert_eq!(
            state.probe(&untouched_pool()).await,
            Readiness::NotReady(NotReadyReason::ShuttingDown)
        );
    }

    #[tokio::test]
    async fn shutdown_overrides_a_cached_ready_result() {
        let state = ReadinessState::new();
        state.set_initialised();
        *state.0.cache.lock().await = Some(CachedReady { at: Instant::now() });
        state.begin_shutdown();
        assert_eq!(
            state.probe(&untouched_pool()).await,
            Readiness::NotReady(NotReadyReason::ShuttingDown)
        );
    }

    /// Fix round 1, item 2: the cache's "a failure is never cached" property, made
    /// deterministic via `probe_with`'s injectable check -- no real database, no
    /// timing window to race. A prior integration test could only prove this by
    /// polling within an arbitrary window, which cannot actually distinguish "never
    /// cached" from "cached for less than the window observed".
    #[tokio::test]
    async fn a_failure_is_never_cached_so_the_next_call_rechecks() {
        let state = ReadinessState::new();
        state.set_initialised();

        let first = state
            .probe_with(|| async { Readiness::NotReady(NotReadyReason::Database) })
            .await;
        assert_eq!(first, Readiness::NotReady(NotReadyReason::Database));

        // If the failure above had been cached, this would still report
        // `NotReady(Database)` despite the check function now reporting `Ready`.
        let second = state.probe_with(|| async { Readiness::Ready }).await;
        assert_eq!(
            second,
            Readiness::Ready,
            "a failed probe must never be cached: the very next call has to re-check \
             and can report Ready immediately"
        );
    }

    /// The other half of the same property: a `Ready` result *is* cached, so an
    /// immediately following call does not re-check at all.
    #[tokio::test]
    async fn a_success_is_cached_for_the_ttl() {
        let state = ReadinessState::new();
        state.set_initialised();

        let first = state.probe_with(|| async { Readiness::Ready }).await;
        assert_eq!(first, Readiness::Ready);

        // The check function here would report `NotReady` if it ran -- the cached
        // `Ready` from above must win instead.
        let second = state
            .probe_with(|| async { Readiness::NotReady(NotReadyReason::Database) })
            .await;
        assert_eq!(
            second,
            Readiness::Ready,
            "a successful probe must be cached for CACHE_TTL"
        );
    }

    /// Fix round 1, item 5: `shutting_down` is re-read after the check completes, so
    /// a probe already in flight when shutdown begins cannot still report `Ready`.
    #[tokio::test]
    async fn shutdown_during_a_slow_check_overrides_its_ready_result() {
        let state = ReadinessState::new();
        state.set_initialised();

        let inner = state.clone();
        let result = state
            .probe_with(|| async move {
                // Simulates shutdown beginning while a check is already in flight.
                inner.begin_shutdown();
                Readiness::Ready
            })
            .await;

        assert_eq!(result, Readiness::NotReady(NotReadyReason::ShuttingDown));
    }
}
