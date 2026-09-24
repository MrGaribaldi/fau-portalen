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
}

impl SourceClient {
    pub fn new(urls: SourceUrls) -> Result<Self, SourceError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("fau-register/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| SourceError::new(Source::Nsr, SourceErrorKind::Transport))?;
        Ok(Self { http, urls })
    }

    async fn get(
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
        let mut units = Vec::new();
        let mut page_count = None;
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
            let parsed = nsr::parse_list_page(&body)?;
            let count = *page_count.get_or_insert(parsed.page_count);
            if parsed.units.is_empty() {
                return Ok(units);
            }
            units.extend(parsed.units);
            if page >= count {
                return Ok(units);
            }
        }
        Err(SourceError::new(Source::Nsr, SourceErrorKind::TooManyPages))
    }

    pub async fn nsr_units_in(
        &self,
        municipality_number: &str,
    ) -> Result<Vec<NsrListItem>, SourceError> {
        let url = format!("{}/v4/enheter/kommune/{municipality_number}", self.urls.nsr);
        Ok(nsr::parse_list_page(&self.get(Source::Nsr, &url, &[], false).await?)?.units)
    }

    pub async fn nsr_unit(&self, orgnr: &str) -> Result<NsrUnit, SourceError> {
        let url = format!("{}/v4/enhet/{orgnr}", self.urls.nsr);
        nsr::parse_unit(&self.get(Source::Nsr, &url, &[], false).await?)
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
        let mut file = tokio::fs::File::create(dest).await.map_err(io)?;
        let mut written = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(transport)? {
            file.write_all(&chunk).await.map_err(io)?;
            written += chunk.len() as u64;
        }
        file.flush().await.map_err(io)?;
        Ok(written)
    }
}
