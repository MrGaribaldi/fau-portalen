//! `/health/live` and `/health/ready` (design section 9).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::readiness::{NotReadyReason, Readiness};

use super::AppState;

/// Touches the process only -- no database, mail, RabbitMQ, S3 or OTLP -- so that a
/// dependency outage can never turn into a restart loop. Unauthenticated, so this
/// carries no information beyond "the process is alive" -- in particular no
/// service-version header; the service version is a log field
/// (`telemetry::JsonLineLayer`), not exposed here.
pub async fn live() -> StatusCode {
    StatusCode::OK
}

/// The `/health/ready` wire body. Minimal and leaks nothing internal (ADR-001): no
/// database version, no address, no error text -- `reason` is one of a fixed,
/// small set of words, never derived from anything the database or a caller
/// supplied.
#[derive(Serialize)]
#[serde(tag = "status")]
enum ReadyBody {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "not_ready")]
    NotReady { reason: &'static str },
}

/// The fixed, safe word for each [`NotReadyReason`] -- never the schema-contract
/// version numbers that variant also carries internally, which stay out of the
/// response body on purpose.
fn reason_word(reason: NotReadyReason) -> &'static str {
    match reason {
        NotReadyReason::Initialising => "initialising",
        NotReadyReason::Database => "database",
        NotReadyReason::SchemaContract { .. } => "schema_contract",
        NotReadyReason::ShuttingDown => "shutting_down",
    }
}

/// Delegates to [`crate::readiness::ReadinessState::probe`]: local initialisation,
/// a database check bounded to one second, and the schema contract, cached briefly
/// on success and never on failure.
pub async fn ready(State(state): State<AppState>) -> Response {
    match state.readiness.probe(&state.pool).await {
        Readiness::Ready => (StatusCode::OK, Json(ReadyBody::Ready)).into_response(),
        Readiness::NotReady(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadyBody::NotReady {
                reason: reason_word(reason),
            }),
        )
            .into_response(),
    }
}
