//! §5.2 steps 1-3: Kartverket, then SSB, then the NSR list and the detail of every unit the
//! run needs. Nothing here logs a URL, a body or an address.

use std::collections::BTreeSet;
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

/// Everything one run fetched.
pub(super) struct Fetched {
    pub municipalities: Vec<MunicipalityRecord>,
    pub code_changes: Vec<CodeChange>,
    /// Each unit with the raw bytes it was parsed from, sorted by orgnr.
    pub units: Vec<(NsrUnit, Vec<u8>)>,
    /// Register orgnrs NSR's list no longer carries: left untouched, and only counted.
    pub absent_from_list: usize,
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
) -> Result<Fetched, SourceError> {
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
/// a partial detail pass is never planned from (§5.3).
async fn details(
    client: Arc<SourceClient>,
    orgnrs: Vec<String>,
) -> Result<Vec<(NsrUnit, Vec<u8>)>, SourceError> {
    let queue = Arc::new(Mutex::new(orgnrs.into_iter()));
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..DETAIL_FETCHES_IN_PARALLEL {
        let (client, queue) = (client.clone(), queue.clone());
        workers.spawn(async move {
            let mut fetched = Vec::new();
            loop {
                let next = queue.lock().expect("the detail queue lock").next();
                let Some(orgnr) = next else {
                    return Ok::<_, SourceError>(fetched);
                };
                fetched.push(client.nsr_unit_with_payload(&orgnr).await?);
            }
        });
    }
    let mut all = Vec::new();
    while let Some(joined) = workers.join_next().await {
        all.extend(joined.expect("a detail fetch worker panicked")?);
    }
    all.sort_by(|a, b| a.0.orgnr.cmp(&b.0.orgnr));
    Ok(all)
}
