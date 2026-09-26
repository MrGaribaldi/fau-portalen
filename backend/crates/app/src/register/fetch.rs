//! §5.2 steps 1-3: Kartverket, then SSB, then the NSR list and the detail of every unit the
//! run needs. Nothing here logs a URL, a body or an address.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use fau_domain::register::source::{CodeChange, MunicipalityRecord, NsrUnit};
use fau_domain::register::sync::{RegisterSnapshot, RunKind, SchoolStatus};
use fau_register_sources::client::{SourceClient, SourceUrls};
use fau_register_sources::SourceError;
use jiff::civil::Date;
use url::Url;
use uuid::Uuid;

use crate::config::RegisterConfig;

/// §5.2 step 3: "about a minute at four in parallel".
const DETAIL_FETCHES_IN_PARALLEL: usize = 4;

/// The detail pass logs a progress line every this many units (final review, item 11).
const PROGRESS_EVERY: usize = 500;

/// Everything one run fetched.
pub(super) struct Fetched {
    pub municipalities: Vec<MunicipalityRecord>,
    pub code_changes: Vec<CodeChange>,
    /// Each unit with the raw bytes it was parsed from, sorted by orgnr.
    pub units: Vec<(NsrUnit, Vec<u8>)>,
    /// Register orgnrs NSR's list no longer carries: left untouched, and only counted.
    pub absent_from_list: usize,
}

/// A genuine source error, or one of [`DETAIL_FETCHES_IN_PARALLEL`]'s workers panicking (or
/// being cancelled) instead of returning one. Either way the run must still record `failed`
/// and exit normally -- never let the panic itself reach the top of the process and crash it
/// with an unhandled exit code, leaving the run row unfinished.
#[derive(Debug, thiserror::Error)]
pub(super) enum FetchError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error("an internal detail-fetch worker panicked")]
    WorkerPanicked,
}

pub(super) fn source_urls(config: &RegisterConfig) -> SourceUrls {
    let base = |url: &Option<Url>, default: String| {
        url.as_ref()
            .map_or(default, |u| u.as_str().trim_end_matches('/').to_owned())
    };
    let production = SourceUrls::production();
    SourceUrls {
        nsr: base(&config.nsr_url, production.nsr),
        kartverket: base(&config.kartverket_url, production.kartverket),
        ssb: base(&config.ssb_url, production.ssb),
        // Part 5 reads Brreg; this run never does.
        brreg: production.brreg,
    }
}

/// Fetches in the Handover's order. A seed is passed no code changes at all; a sync reads
/// SSB over the fixed lookback from `ssb_from` (the seed date) to today. The NSR detail pass
/// covers every active grunnskole in the list plus every orgnr of a school the register
/// holds open, once each.
pub(super) async fn fetch(
    client: Arc<SourceClient>,
    snapshot: &RegisterSnapshot<Uuid>,
    kind: RunKind,
    ssb_from: Option<Date>,
    today: Date,
) -> Result<Fetched, FetchError> {
    let municipalities = client.municipalities().await?;
    let code_changes = match (kind, ssb_from) {
        (RunKind::Sync, Some(from)) => client.code_changes(from, today).await?,
        _ => Vec::new(),
    };

    let list = client.nsr_all_units().await?;
    let listed: BTreeSet<&str> = list.iter().map(|u| u.orgnr.as_str()).collect();
    let mut wanted: BTreeSet<String> = list
        .iter()
        .filter(|u| u.is_active && u.is_primary_school)
        .map(|u| u.orgnr.clone())
        .collect();
    let mut absent_from_list = 0;
    for school in &snapshot.schools {
        if school.status == SchoolStatus::Closed {
            continue;
        }
        if let Some(orgnr) = &school.orgnr {
            if listed.contains(orgnr.as_str()) {
                wanted.insert(orgnr.clone());
            } else {
                absent_from_list += 1;
            }
        }
    }

    let units = details(client, wanted.into_iter().collect()).await?;
    Ok(Fetched {
        municipalities,
        code_changes,
        units,
        absent_from_list,
    })
}

/// [`DETAIL_FETCHES_IN_PARALLEL`] workers draining one queue. The first error ends the run:
/// a partial detail pass is never planned from (§5.3), so the other workers stop taking new
/// orgnrs once one has failed. The failure is logged with its orgnr (a public identifier)
/// and the error's source and kind, never a URL or a body. A worker panicking (or being
/// cancelled) is reported the same way, rather than panicking this task too -- see
/// [`FetchError`]. Progress goes to the log every [`PROGRESS_EVERY`] units, in counts only.
async fn details(
    client: Arc<SourceClient>,
    orgnrs: Vec<String>,
) -> Result<Vec<(NsrUnit, Vec<u8>)>, FetchError> {
    let total = orgnrs.len();
    let queue = Arc::new(Mutex::new(orgnrs.into_iter()));
    let failed = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicUsize::new(0));
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..DETAIL_FETCHES_IN_PARALLEL {
        let (client, queue) = (client.clone(), queue.clone());
        let (failed, done) = (failed.clone(), done.clone());
        workers.spawn(async move {
            let mut fetched = Vec::new();
            loop {
                if failed.load(Ordering::SeqCst) {
                    return Ok(fetched);
                }
                // A previous worker panicking while holding this lock is not credible (the
                // critical section is a plain iterator advance, nothing that can panic), but
                // recovering a poisoned lock instead of `.expect`ing it is free, so it costs
                // nothing to never add a second panic path here.
                let next = queue
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .next();
                let Some(orgnr) = next else {
                    return Ok::<_, SourceError>(fetched);
                };
                match client.nsr_unit_with_payload(&orgnr).await {
                    Ok(unit) => fetched.push(unit),
                    Err(e) => {
                        failed.store(true, Ordering::SeqCst);
                        tracing::error!(
                            orgnr = %orgnr,
                            source = ?e.source,
                            kind = ?e.kind,
                            "NSR detail fetch failed"
                        );
                        return Err(e);
                    }
                }
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                if progress_due(n) {
                    tracing::info!(fetched = n, total, "NSR detail pass progress");
                }
            }
        });
    }
    let mut all = Vec::new();
    let mut first_error = None;
    while let Some(joined) = workers.join_next().await {
        match joined {
            Ok(Ok(items)) => all.extend(items),
            Ok(Err(e)) => {
                first_error.get_or_insert(FetchError::Source(e));
            }
            Err(_join_error) => return Err(FetchError::WorkerPanicked),
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    tracing::info!(fetched = all.len(), total, "NSR detail pass finished");
    all.sort_by(|a, b| a.0.orgnr.cmp(&b.0.orgnr));
    Ok(all)
}

fn progress_due(fetched: usize) -> bool {
    fetched.is_multiple_of(PROGRESS_EVERY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_logged_every_500_units() {
        let due: Vec<usize> = (1..=1600).filter(|n| progress_due(*n)).collect();
        assert_eq!(due, [500, 1000, 1500]);
    }
}
