//! The client against a real HTTP server on 127.0.0.1, serving the recorded fixtures: real
//! sockets, real reqwest, no mocks.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::Router;
use fau_register_sources::client::{SourceClient, SourceUrls};
use fau_register_sources::{Source, SourceErrorKind};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// NSR paging served from one recorded unit list, three units per page, so the client has to
/// follow several pages and stop at an empty one.
fn paged_nsr(q: &std::collections::HashMap<String, String>) -> Vec<u8> {
    let list: serde_json::Value =
        serde_json::from_slice(&fixture("nsr/kommune-3201.json")).unwrap();
    let all = list["EnhetListe"].as_array().unwrap()[..7].to_vec();
    let page: usize = q["sidenummer"].parse().unwrap();
    let chunk: Vec<_> = all
        .chunks(3)
        .nth(page - 1)
        .map(|c| c.to_vec())
        .unwrap_or_default();
    serde_json::to_vec(&serde_json::json!({
        "Sidenummer": page, "AntallPerSide": 3, "AntallSider": 3, "TotaltAntallEnheter": 7, "EnhetListe": chunk
    }))
    .unwrap()
}

async fn serve(hits: Arc<AtomicUsize>) -> String {
    let app = Router::new()
        .route("/nsr/v4/enheter", get(|Query(q): Query<std::collections::HashMap<String, String>>| async move { paged_nsr(&q) }))
        .route("/nsr/v4/enheter/kommune/{nr}", get(|Path(nr): Path<String>| async move {
            if nr == "3201" { (StatusCode::OK, fixture("nsr/kommune-3201.json")) } else { (StatusCode::NOT_FOUND, Vec::new()) }
        }))
        .route("/nsr/v4/enhet/{orgnr}", get(|Path(o): Path<String>| async move { fixture(&format!("nsr/enhet-{o}.json")) }))
        .route("/kv/fylkerkommuner", get(|| async { fixture("kartverket/fylkerkommuner.json") }))
        .route("/ssb/classifications/131/changes", get(|Query(q): Query<std::collections::HashMap<String, String>>, h: HeaderMap| async move {
            assert_eq!(h.get("accept").unwrap(), "application/json", "SSB returns XML without it");
            assert_eq!((q["from"].as_str(), q["to"].as_str()), ("2023-12-01", "2024-01-31"));
            fixture("ssb/changes-2024.json")
        }))
        .route("/brreg/enheter/lastned", get(move || {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                std::io::Write::write_all(&mut enc, &fixture("brreg/enheter-sample.json")).unwrap();
                enc.finish().unwrap()
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn client() -> (SourceClient, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = serve(hits.clone()).await;
    let urls = SourceUrls {
        nsr: format!("{base}/nsr"),
        kartverket: format!("{base}/kv"),
        ssb: format!("{base}/ssb"),
        brreg: format!("{base}/brreg"),
    };
    (SourceClient::new(urls).unwrap(), hits)
}

#[tokio::test]
async fn nsr_paging_follows_every_page() {
    let (c, _) = client().await;
    let units = c.nsr_all_units().await.unwrap();
    assert_eq!(units.len(), 7, "3 + 3 + 1 across three pages");
}

#[tokio::test]
async fn nsr_municipality_list_and_detail() {
    let (c, _) = client().await;
    assert_eq!(c.nsr_units_in("3201").await.unwrap().len(), 218);
    assert_eq!(c.nsr_unit("974552124").await.unwrap().name, "Hosle skole");
}

#[tokio::test]
async fn an_http_error_status_is_reported_with_its_source() {
    let (c, _) = client().await;
    let err = c.nsr_units_in("9999").await.unwrap_err();
    assert_eq!(
        (err.source, err.kind),
        (Source::Nsr, SourceErrorKind::Status { status: 404 })
    );
}

#[tokio::test]
async fn municipalities_and_changes() {
    let (c, _) = client().await;
    assert_eq!(c.municipalities().await.unwrap().len(), 357);
    let from = "2023-12-01".parse().unwrap();
    let to = "2024-01-31".parse().unwrap();
    assert_eq!(c.code_changes(from, to).await.unwrap().len(), 118);
}

#[tokio::test]
async fn brreg_bulk_download_lands_on_disk_and_streams() {
    let (c, hits) = client().await;
    let dir = std::env::temp_dir().join(format!("fau-brreg-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("enheter.json.gz");
    let written = c.download_brreg_bulk(&dest).await.unwrap();
    assert!(written > 0);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let stats =
        fau_register_sources::brreg::for_each_fau(std::fs::File::open(&dest).unwrap(), |_| {})
            .unwrap();
    assert_eq!(stats.faus, 8);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn an_unreachable_source_is_a_transport_error() {
    let dead = SourceUrls {
        nsr: "http://127.0.0.1:9/nsr".into(),
        kartverket: "http://127.0.0.1:9/kv".into(),
        ssb: "http://127.0.0.1:9/ssb".into(),
        brreg: "http://127.0.0.1:9/brreg".into(),
    };
    let err = SourceClient::new(dead)
        .unwrap()
        .municipalities()
        .await
        .unwrap_err();
    assert_eq!(
        (err.source, err.kind),
        (Source::Kartverket, SourceErrorKind::Transport)
    );
}

#[test]
fn production_urls_are_the_documented_ones() {
    let u = SourceUrls::production();
    assert_eq!(u.nsr, "https://data-nsr.udir.no");
    assert_eq!(u.kartverket, "https://api.kartverket.no/kommuneinfo/v1");
    assert_eq!(u.ssb, "https://data.ssb.no/api/klass/v1");
    assert_eq!(u.brreg, "https://data.brreg.no/enhetsregisteret/api");
}
