//! A small client for the four sources. Base URLs are parameters, so tests point it at a
//! local server, and production uses [`SourceUrls::production`].

use std::path::Path;
use std::time::Duration;

use fau_domain::register::source::{CodeChange, MunicipalityRecord, NsrUnit};
use jiff::civil::Date;
use tokio::io::AsyncWriteExt;

use crate::error::{Source, SourceError, SourceErrorKind};
use crate::nsr::NsrListItem;
use crate::{kartverket, nsr, ssb};

/// NSR pages are requested 1,000 at a time. 19 pages cover the whole register today (§2.1).
const NSR_PAGE_SIZE: u32 = 1000;
/// A hard stop, so an upstream paging bug cannot loop forever.
const NSR_MAX_PAGES: u32 = 100;
const BRREG_TIMEOUT: Duration = Duration::from_secs(600);

/// The waits before the second and the third attempt of a request (final review, item 2):
/// three attempts in total. Only a 5xx status or a transport error is retried; a 4xx or a
/// response that does not parse would fail the same way again.
pub const DEFAULT_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceUrls {
    pub nsr: String,
    pub kartverket: String,
    pub ssb: String,
    pub brreg: String,
}

impl SourceUrls {
    pub fn production() -> Self {
        Self {
            nsr: "https://data-nsr.udir.no".into(),
            kartverket: "https://api.kartverket.no/kommuneinfo/v1".into(),
            ssb: "https://data.ssb.no/api/klass/v1".into(),
            brreg: "https://data.brreg.no/enhetsregisteret/api".into(),
        }
    }
}

pub struct SourceClient {
    http: reqwest::Client,
    urls: SourceUrls,
    retry_delays: [Duration; 2],
}

impl SourceClient {
    pub fn new(urls: SourceUrls) -> Result<Self, SourceError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("fau-register/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| SourceError::new(Source::Nsr, SourceErrorKind::Transport))?;
        Ok(Self {
            http,
            urls,
            retry_delays: DEFAULT_RETRY_DELAYS,
        })
    }

    /// Replaces [`DEFAULT_RETRY_DELAYS`], e.g. so a test need not wait five seconds.
    pub fn with_retry_delays(mut self, delays: [Duration; 2]) -> Self {
        self.retry_delays = delays;
        self
    }

    /// One GET, retried after each of `retry_delays` on a 5xx or a transport error. Every
    /// other failure, and the last attempt's, is returned as it is. The body is only returned
    /// once it has been read whole, so a parse error downstream is never retried.
    async fn get(
        &self,
        source: Source,
        url: &str,
        query: &[(&str, String)],
        accept_json: bool,
    ) -> Result<Vec<u8>, SourceError> {
        let mut delays = self.retry_delays.iter();
        loop {
            match self.get_once(source, url, query, accept_json).await {
                Err(e) if retryable(&e.kind) => match delays.next() {
                    Some(delay) => {
                        // The source and a fixed kind only: never the URL or a body.
                        tracing::warn!(
                            source = ?e.source,
                            kind = ?e.kind,
                            retry_in_ms = delay.as_millis() as u64,
                            "source request failed, retrying"
                        );
                        tokio::time::sleep(*delay).await;
                    }
                    None => return Err(e),
                },
                result => return result,
            }
        }
    }

    async fn get_once(
        &self,
        source: Source,
        url: &str,
        query: &[(&str, String)],
        accept_json: bool,
    ) -> Result<Vec<u8>, SourceError> {
        let mut req = self.http.get(url).query(query);
        if accept_json {
            req = req.header(reqwest::header::ACCEPT, "application/json");
        }
        let resp = req
            .send()
            .await
            .map_err(|_| SourceError::new(source, SourceErrorKind::Transport))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(SourceError::new(
                source,
                SourceErrorKind::Status {
                    status: status.as_u16(),
                },
            ));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|_| SourceError::new(source, SourceErrorKind::Transport))
    }

    pub async fn nsr_all_units(&self) -> Result<Vec<NsrListItem>, SourceError> {
        let url = format!("{}/v4/enheter", self.urls.nsr);
        let mut seen = std::collections::HashSet::new();
        let mut units: Vec<NsrListItem> = Vec::new();
        let mut page_count = None;
        let mut total = None;
        for page in 1..=NSR_MAX_PAGES {
            let body = self
                .get(
                    Source::Nsr,
                    &url,
                    &[
                        ("sidenummer", page.to_string()),
                        ("antallperside", NSR_PAGE_SIZE.to_string()),
                    ],
                    false,
                )
                .await?;
            let parsed = nsr::parse_paged_list_page(&body)?;
            let count = *page_count.get_or_insert(parsed.page_count);
            let expected_total = *total.get_or_insert(parsed.total);
            if parsed.units.is_empty() {
                // An empty page before the claimed page count is a truncated run, never a
                // silent partial list (§5.3).
                if page < count {
                    return Err(SourceError::new(
                        Source::Nsr,
                        SourceErrorKind::IncompletePaging,
                    ));
                }
                return Self::finished_paging(units, expected_total);
            }
            for item in parsed.units {
                if seen.insert(item.orgnr.clone()) {
                    units.push(item);
                }
            }
            if page >= count {
                return Self::finished_paging(units, expected_total);
            }
        }
        Err(SourceError::new(Source::Nsr, SourceErrorKind::TooManyPages))
    }

    /// The deduped count must equal the claimed total, or the run is incomplete (§5.3).
    fn finished_paging(
        units: Vec<NsrListItem>,
        total: u32,
    ) -> Result<Vec<NsrListItem>, SourceError> {
        if units.len() as u32 != total {
            return Err(SourceError::new(
                Source::Nsr,
                SourceErrorKind::IncompletePaging,
            ));
        }
        Ok(units)
    }

    pub async fn nsr_units_in(
        &self,
        municipality_number: &str,
    ) -> Result<Vec<NsrListItem>, SourceError> {
        let url = format!("{}/v4/enheter/kommune/{municipality_number}", self.urls.nsr);
        Ok(nsr::parse_list_page(&self.get(Source::Nsr, &url, &[], false).await?)?.units)
    }

    pub async fn nsr_unit(&self, orgnr: &str) -> Result<NsrUnit, SourceError> {
        self.nsr_unit_with_payload(orgnr)
            .await
            .map(|(unit, _)| unit)
    }

    /// Like [`Self::nsr_unit`], but also hands back the raw bytes it parsed, so a caller can
    /// store the payload and its hash for provenance (§4.3).
    pub async fn nsr_unit_with_payload(
        &self,
        orgnr: &str,
    ) -> Result<(NsrUnit, Vec<u8>), SourceError> {
        let url = format!("{}/v4/enhet/{orgnr}", self.urls.nsr);
        let body = self.get(Source::Nsr, &url, &[], false).await?;
        let unit = nsr::parse_unit(&body)?;
        Ok((unit, body))
    }

    pub async fn municipalities(&self) -> Result<Vec<MunicipalityRecord>, SourceError> {
        let url = format!("{}/fylkerkommuner", self.urls.kartverket);
        kartverket::parse_municipalities(&self.get(Source::Kartverket, &url, &[], false).await?)
    }

    pub async fn code_changes(&self, from: Date, to: Date) -> Result<Vec<CodeChange>, SourceError> {
        let url = format!("{}/classifications/131/changes", self.urls.ssb);
        let query = [("from", from.to_string()), ("to", to.to_string())];
        ssb::parse_changes(&self.get(Source::Ssb, &url, &query, true).await?)
    }

    /// Streams the gzipped bulk file to `dest` (about 210 MB) without holding it in memory.
    /// Written first to `dest` with `.partial` appended, renamed to `dest` only once the write
    /// has flushed successfully. On any error, the final rename's included, the partial file
    /// is removed (best effort) and `dest` is never created or overwritten.
    pub async fn download_brreg_bulk(&self, dest: &Path) -> Result<u64, SourceError> {
        let url = format!("{}/enheter/lastned", self.urls.brreg);
        let transport = |_| SourceError::new(Source::Brreg, SourceErrorKind::Transport);
        let mut resp = self
            .http
            .get(&url)
            .timeout(BRREG_TIMEOUT)
            .send()
            .await
            .map_err(transport)?;
        if !resp.status().is_success() {
            return Err(SourceError::new(
                Source::Brreg,
                SourceErrorKind::Status {
                    status: resp.status().as_u16(),
                },
            ));
        }
        let io = |_| SourceError::new(Source::Brreg, SourceErrorKind::Io);
        let partial = partial_path(dest);
        let result: Result<u64, SourceError> = async {
            let mut file = tokio::fs::File::create(&partial).await.map_err(io)?;
            let mut written = 0u64;
            while let Some(chunk) = resp.chunk().await.map_err(transport)? {
                file.write_all(&chunk).await.map_err(io)?;
                written += chunk.len() as u64;
            }
            file.flush().await.map_err(io)?;
            Ok(written)
        }
        .await;
        // The rename is the last step that can fail, so it is covered by the same cleanup:
        // a `.partial` file never outlives a failed download (part 2's carried-over gap).
        let result = match result {
            Ok(written) => tokio::fs::rename(&partial, dest)
                .await
                .map(|()| written)
                .map_err(io),
            Err(err) => Err(err),
        };
        if result.is_err() {
            let _ = tokio::fs::remove_file(&partial).await;
        }
        result
    }
}

/// A 5xx or a transport failure may pass; a 4xx, a parse error or anything else will not.
fn retryable(kind: &SourceErrorKind) -> bool {
    match kind {
        SourceErrorKind::Status { status } => (500..600).contains(status),
        SourceErrorKind::Transport => true,
        _ => false,
    }
}

/// `dest` with `.partial` appended to its file name, e.g. `enheter.json.gz` becomes
/// `enheter.json.gz.partial`.
fn partial_path(dest: &Path) -> std::path::PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    dest.with_file_name(name)
}
