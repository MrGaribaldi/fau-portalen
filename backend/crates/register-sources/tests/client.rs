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

/// NSR paging served from one recorded unit list, three units per page across three pages
/// (3 + 3 + 1 = 7 units), so the client has to follow every page and stop once it reaches the
/// claimed page count (`AntallSider: 3`), not because any page comes back empty.
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

/// A minimal, validly-shaped `/v4/enheter` list item (only the fields `parse_paged_list_page`
/// requires).
fn nsr_list_item(orgnr: &str) -> serde_json::Value {
    serde_json::json!({
        "Organisasjonsnummer": orgnr,
        "Navn": "Test skole",
        "Kommunenummer": "0301",
        "ErAktiv": true,
        "ErSkole": true,
        "ErGrunnskole": true,
        "DatoEndret": null,
    })
}

/// A client whose NSR base URL is `serve_pages`'s server and whose other sources are never
/// dialled, so a paging test can serve exactly the pages it wants to.
async fn nsr_only_client(pages: Vec<serde_json::Value>) -> SourceClient {
    let pages = Arc::new(pages);
    let app = Router::new().route(
        "/v4/enheter",
        get(
            move |Query(q): Query<std::collections::HashMap<String, String>>| {
                let pages = pages.clone();
                async move {
                    let page: usize = q["sidenummer"].parse().unwrap();
                    let body = &pages[page - 1];
                    serde_json::to_vec(body).unwrap()
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let urls = SourceUrls {
        nsr: format!("http://{addr}"),
        kartverket: "http://127.0.0.1:9/unused".into(),
        ssb: "http://127.0.0.1:9/unused".into(),
        brreg: "http://127.0.0.1:9/unused".into(),
    };
    SourceClient::new(urls).unwrap()
}

#[tokio::test]
async fn an_empty_page_before_the_claimed_page_count_is_incomplete_paging() {
    let page1 = serde_json::json!({
        "Sidenummer": 1, "AntallSider": 3, "TotaltAntallEnheter": 3,
        "EnhetListe": [nsr_list_item("1"), nsr_list_item("2"), nsr_list_item("3")],
    });
    let page2 = serde_json::json!({
        "Sidenummer": 2, "AntallSider": 3, "TotaltAntallEnheter": 3,
        "EnhetListe": [],
    });
    let c = nsr_only_client(vec![page1, page2]).await;
    let err = c.nsr_all_units().await.unwrap_err();
    assert_eq!(
        (err.source, err.kind),
        (Source::Nsr, SourceErrorKind::IncompletePaging)
    );
}

#[tokio::test]
async fn a_claimed_total_higher_than_the_units_actually_seen_is_incomplete_paging() {
    let page1 = serde_json::json!({
        "Sidenummer": 1, "AntallSider": 3, "TotaltAntallEnheter": 9,
        "EnhetListe": [nsr_list_item("1"), nsr_list_item("2"), nsr_list_item("3")],
    });
    let page2 = serde_json::json!({
        "Sidenummer": 2, "AntallSider": 3, "TotaltAntallEnheter": 9,
        "EnhetListe": [nsr_list_item("4"), nsr_list_item("5"), nsr_list_item("6")],
    });
    let page3 = serde_json::json!({
        "Sidenummer": 3, "AntallSider": 3, "TotaltAntallEnheter": 9,
        "EnhetListe": [nsr_list_item("7")],
    });
    let c = nsr_only_client(vec![page1, page2, page3]).await;
    let err = c.nsr_all_units().await.unwrap_err();
    assert_eq!(
        (err.source, err.kind),
        (Source::Nsr, SourceErrorKind::IncompletePaging)
    );
}

#[tokio::test]
async fn a_unit_repeated_across_pages_is_deduped_by_orgnr() {
    let page1 = serde_json::json!({
        "Sidenummer": 1, "AntallSider": 2, "TotaltAntallEnheter": 7,
        "EnhetListe": [nsr_list_item("1"), nsr_list_item("2"), nsr_list_item("3")],
    });
    // "1" repeats here: a naive concatenation would total 8 units, not the claimed 7.
    let page2 = serde_json::json!({
        "Sidenummer": 2, "AntallSider": 2, "TotaltAntallEnheter": 7,
        "EnhetListe": [
            nsr_list_item("1"), nsr_list_item("4"), nsr_list_item("5"),
            nsr_list_item("6"), nsr_list_item("7"),
        ],
    });
    let c = nsr_only_client(vec![page1, page2]).await;
    let units = c.nsr_all_units().await.unwrap();
    assert_eq!(
        units.len(),
        7,
        "the repeated unit \"1\" is kept once, from the first page"
    );
}

#[tokio::test]
async fn a_first_page_missing_antall_sider_is_a_parse_error() {
    let page1 = serde_json::json!({
        "Sidenummer": 1, "TotaltAntallEnheter": 1,
        "EnhetListe": [nsr_list_item("1")],
    });
    let c = nsr_only_client(vec![page1]).await;
    let err = c.nsr_all_units().await.unwrap_err();
    assert_eq!(err.source, Source::Nsr);
    assert!(
        matches!(err.kind, SourceErrorKind::Parse { .. }),
        "{:?}",
        err.kind
    );
}

#[tokio::test]
async fn nsr_municipality_list_and_detail() {
    let (c, _) = client().await;
    assert_eq!(c.nsr_units_in("3201").await.unwrap().len(), 218);
    assert_eq!(c.nsr_unit("974552124").await.unwrap().name, "Hosle skole");
}

#[tokio::test]
async fn nsr_unit_with_payload_returns_the_bytes_it_parsed() {
    let (c, _) = client().await;
    let (unit, bytes) = c.nsr_unit_with_payload("974552124").await.unwrap();
    assert_eq!(unit.name, "Hosle skole");
    assert_eq!(bytes, fixture("nsr/enhet-974552124.json"));
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
    assert!(
        !dir.join("enheter.json.gz.partial").exists(),
        "no .partial file should remain after a successful download"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_failed_bulk_download_leaves_no_dest_and_no_partial_file() {
    let app = axum::Router::new().route(
        "/brreg/enheter/lastned",
        axum::routing::get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let urls = SourceUrls {
        nsr: "http://127.0.0.1:9/unused".into(),
        kartverket: "http://127.0.0.1:9/unused".into(),
        ssb: "http://127.0.0.1:9/unused".into(),
        brreg: format!("http://{addr}/brreg"),
    };
    let c = SourceClient::new(urls).unwrap();
    let dir = std::env::temp_dir().join(format!("fau-brreg-fail-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("enheter.json.gz");
    let err = c.download_brreg_bulk(&dest).await.unwrap_err();
    assert_eq!(
        (err.source, err.kind),
        (Source::Brreg, SourceErrorKind::Status { status: 500 })
    );
    assert!(!dest.exists(), "dest must never be created on failure");
    assert!(
        !dir.join("enheter.json.gz.partial").exists(),
        "the partial file must not survive a failed download"
    );
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
