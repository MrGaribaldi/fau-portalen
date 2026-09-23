//! `/health/live` and `/health/ready` (design section 9).

use axum::extract::State;
use axum::http::StatusCode;

use super::AppState;

/// Touches the process only -- no database, mail, RabbitMQ, S3 or OTLP -- so that a
/// dependency outage can never turn into a restart loop. Unauthenticated, so this
/// carries no information beyond "the process is alive" -- in particular no
/// service-version header; `AppState.version` is read from the startup banner
/// instead (`main.rs`), not exposed here.
pub async fn live() -> StatusCode {
    StatusCode::OK
}

/// Delegates to [`crate::readiness::ReadinessState`]. Today's stub is always
/// ready; Task 9 adds the real database and schema-contract check behind the same
/// interface.
pub async fn ready(State(state): State<AppState>) -> StatusCode {
    if state.readiness.is_ready().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
