//! NSR staging (docs/school-register-design.md §4.3, §5.2 step 4): the last payload seen
//! per orgnr, with its SHA-256 and scope, for provenance. NSR only: Brreg payloads carry
//! addresses and are never stored.

use fau_domain::register::scope::{classify, ScopeDecision};
use fau_domain::register::source::NsrUnit;
use fau_domain::time::Moment;
use jiff::Timestamp;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use super::error::RegisterError;
use super::sql::ts_param;

/// One fetched NSR detail: the raw bytes the unit was parsed from, and what the register
/// keeps beside them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrPayload {
    pub orgnr: String,
    pub body: Vec<u8>,
    pub changed_at: Option<Timestamp>,
    /// The filter's own verdict (§2.3), before any operator override.
    pub scope: ScopeDecision,
}

impl NsrPayload {
    pub fn new(unit: &NsrUnit, body: Vec<u8>) -> Self {
        NsrPayload {
            orgnr: unit.orgnr.clone(),
            body,
            changed_at: unit.changed_at,
            scope: classify(&unit.scope_facts()),
        }
    }
}

/// Upserts every payload by `(source, external_id)`, then marks every school whose orgnr was
/// fetched as seen at `at`. Call it only inside an applied run: a `no_change` run writes
/// nothing but its run row (§5.3).
pub async fn stage_payloads(
    conn: &mut PgConnection,
    payloads: &[NsrPayload],
    at: Moment,
) -> Result<(), RegisterError> {
    let now = ts_param(at.now());
    for p in payloads {
        let json = std::str::from_utf8(&p.body).map_err(|_| RegisterError::PayloadNotUtf8)?;
        let sha: String = Sha256::digest(&p.body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        sqlx::query(
            "insert into register_source_records
               (source, external_id, payload, payload_sha256, source_changed_at, fetched_at,
                in_scope, scope_reason)
             values ('nsr', $1, $2::jsonb, $3, $4::timestamptz, $5::timestamptz, $6, $7)
             on conflict (source, external_id) do update
               set payload = excluded.payload, payload_sha256 = excluded.payload_sha256,
                   source_changed_at = excluded.source_changed_at,
                   fetched_at = excluded.fetched_at, in_scope = excluded.in_scope,
                   scope_reason = excluded.scope_reason",
        )
        .bind(&p.orgnr)
        .bind(json)
        .bind(sha)
        .bind(p.changed_at.map(ts_param))
        .bind(&now)
        .bind(p.scope == ScopeDecision::InScope)
        .bind(p.scope.code())
        .execute(&mut *conn)
        .await?;
    }
    let orgnrs: Vec<&str> = payloads.iter().map(|p| p.orgnr.as_str()).collect();
    sqlx::query(
        "update schools set last_seen_in_source_at = $1::timestamptz where orgnr = any($2)",
    )
    .bind(&now)
    .bind(&orgnrs)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
