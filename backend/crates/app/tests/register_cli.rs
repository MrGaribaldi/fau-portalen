//! `fau register sync` end to end (#3441 part 4): the real binary, as `fau_register`,
//! against a real test database, with every source URL pointed at an in-process server
//! that serves the recorded fixtures in crates/register-sources/tests/fixtures/. No test
//! calls the network.

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use common::register::run_fau_register;
use common::TestDb;
use fau_persistence::register::try_lock;
use serde_json::{json, Value};
use sqlx::PgPool;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../register-sources/tests/fixtures"
);

/// Every NSR unit with a recorded detail.
const ORGNRS: [&str; 13] = [
    "933181995",
    "974552124",
    "974554682",
    "974795655",
    "975270920",
    "986779795",
    "990672938",
    "998245508",
    "998516897",
    "998666783",
    "998670799",
    "999038182",
    "U90099017",
];

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/{path}")).expect("a recorded fixture")
}

/// NSR's `/v4/enheter` for the recorded units, on one page: each detail cut down to the
/// list model's fields.
fn nsr_list() -> Value {
    let items: Vec<Value> = ORGNRS
        .iter()
        .map(|orgnr| {
            let d: Value =
                serde_json::from_slice(&fixture(&format!("nsr/enhet-{orgnr}.json"))).unwrap();
            json!({
                "Organisasjonsnummer": d["Organisasjonsnummer"],
                "Navn": d["Navn"],
                "Kommunenummer": d["Kommune"]["Kommunenummer"],
                "ErAktiv": d["ErAktiv"],
                "ErSkole": d["ErSkole"],
                "ErGrunnskole": d["ErGrunnskole"],
                "DatoEndret": d["DatoEndret"],
            })
        })
        .collect();
    json!({
        "Sidenummer": 1, "AntallPerSide": 1000, "AntallSider": 1,
        "TotaltAntallEnheter": items.len(), "EnhetListe": items,
    })
}

/// The in-process sources. `empty_nsr` makes NSR answer with an empty list; `fail_detail`
/// makes every NSR detail request answer 500 (controller hand-off: a mid-run source failure
/// must still release the advisory lock); `ssb_queries` records every SSB `(from, to)`.
struct Sources {
    base: String,
    empty_nsr: Arc<AtomicBool>,
    fail_detail: Arc<AtomicBool>,
    ssb_queries: Arc<Mutex<Vec<(String, String)>>>,
}

impl Sources {
    async fn start() -> Self {
        let empty_nsr = Arc::new(AtomicBool::new(false));
        let fail_detail = Arc::new(AtomicBool::new(false));
        let ssb_queries = Arc::new(Mutex::new(Vec::new()));
        let (empty, queries) = (empty_nsr.clone(), ssb_queries.clone());
        let fail = fail_detail.clone();
        let app = Router::new()
            .route(
                "/nsr/v4/enheter",
                get(move || {
                    let empty = empty.load(Ordering::SeqCst);
                    async move {
                        let body = if empty {
                            json!({
                                "Sidenummer": 1, "AntallPerSide": 1000, "AntallSider": 1,
                                "TotaltAntallEnheter": 0, "EnhetListe": [],
                            })
                        } else {
                            nsr_list()
                        };
                        serde_json::to_vec(&body).unwrap()
                    }
                }),
            )
            .route(
                "/nsr/v4/enhet/{orgnr}",
                get(move |Path(orgnr): Path<String>| {
                    let fail = fail.load(Ordering::SeqCst);
                    async move {
                        if fail {
                            return (StatusCode::INTERNAL_SERVER_ERROR, Vec::new());
                        }
                        match std::fs::read(format!("{FIXTURES}/nsr/enhet-{orgnr}.json")) {
                            Ok(body) => (StatusCode::OK, body),
                            Err(_) => (StatusCode::NOT_FOUND, Vec::new()),
                        }
                    }
                }),
            )
            .route(
                "/kv/fylkerkommuner",
                get(|| async { fixture("kartverket/fylkerkommuner.json") }),
            )
            .route(
                "/ssb/classifications/131/changes",
                get(move |Query(q): Query<HashMap<String, String>>| {
                    queries
                        .lock()
                        .unwrap()
                        .push((q["from"].clone(), q["to"].clone()));
                    async { fixture("ssb/changes-2026.json") }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Sources {
            base: format!("http://{addr}"),
            empty_nsr,
            fail_detail,
            ssb_queries,
        }
    }

    /// The environment of a run as `database_url`'s role.
    fn env(&self, database_url: &str) -> Vec<(&'static str, String)> {
        vec![
            ("REGISTER_DATABASE_URL", database_url.to_owned()),
            ("REGISTER_NSR_URL", format!("{}/nsr", self.base)),
            ("REGISTER_KARTVERKET_URL", format!("{}/kv", self.base)),
            ("REGISTER_SSB_URL", format!("{}/ssb", self.base)),
            ("LOG_LEVEL", "info".to_owned()),
        ]
    }
}

async fn fau(args: &[&str], env: &[(&'static str, String)]) -> std::process::Output {
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    run_fau_register(args, &env).await
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

async fn count(admin: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("select count(*) from {table}"))
        .fetch_one(admin)
        .await
        .unwrap()
}

async fn runs(admin: &PgPool) -> Vec<(String, Option<String>, Option<String>)> {
    sqlx::query_as(
        "select kind, outcome, abort_reason from register_sync_runs order by started_at, id",
    )
    .fetch_all(admin)
    .await
    .unwrap()
}

async fn outbox_templates(admin: &PgPool) -> Vec<String> {
    sqlx::query_scalar("select template from outbox order by created_at, id")
        .fetch_all(admin)
        .await
        .unwrap()
}

/// Two addresses from the recorded fixtures (Hosle skole's visiting and postal address).
/// Neither may ever reach a log line or the dry-run output.
const ADDRESSES: [&str; 2] = ["Bispeveien", "Postboks 700"];

#[tokio::test]
async fn a_seed_then_a_sync_of_the_same_sources_changes_nothing() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    let seed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(seed.status.code(), Some(0), "{}", stderr(&seed));
    // 357 Kartverket municipalities and Svalbard; the nine in-scope units; the twelve
    // active grunnskoler staged (the inactive 975270920 is never fetched on a seed).
    assert_eq!(count(&admin, "municipalities").await, 358);
    assert_eq!(count(&admin, "schools").await, 9);
    assert_eq!(count(&admin, "register_source_records").await, 12);
    assert_eq!(count(&admin, "register_review_items").await, 0);
    assert_eq!(count(&admin, "audit_events").await, 1);
    assert_eq!(
        runs(&admin).await,
        [("seed".into(), Some("applied".into()), None)]
    );
    let summary: Value =
        sqlx::query_scalar("select params from outbox where template = 'register.seed_summary'")
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(
        (
            &summary["counts"]["municipalities_created"],
            &summary["counts"]["schools_created"]
        ),
        (&json!(358), &json!(9))
    );
    assert!(
        sources.ssb_queries.lock().unwrap().is_empty(),
        "a seed reads no SSB changes"
    );
    let hosle: String = sqlx::query_scalar(
        "select m.slug || '/' || s.slug from schools s
           join municipalities m on m.id = s.municipality_id where s.orgnr = '974552124'",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(hosle, "3201-baerum/hosle-skole");

    let sync = fau(&["sync"], &env).await;
    assert_eq!(sync.status.code(), Some(0), "{}", stderr(&sync));
    assert_eq!(
        runs(&admin).await,
        [
            ("seed".into(), Some("applied".into()), None),
            ("sync".into(), Some("no_change".into()), None),
        ]
    );
    assert_eq!(outbox_templates(&admin).await, ["register.seed_summary"]);
    // The fixed lookback: from the seed's Oslo date to the sync's.
    let dates: Vec<String> = sqlx::query_scalar(
        "select to_char(started_at at time zone 'Europe/Oslo', 'YYYY-MM-DD')
           from register_sync_runs order by started_at",
    )
    .fetch_all(&admin)
    .await
    .unwrap();
    assert_eq!(
        *sources.ssb_queries.lock().unwrap(),
        [(dates[0].clone(), dates[1].clone())]
    );

    for out in [&seed, &sync] {
        let log = stderr(out);
        assert!(
            log.lines()
                .all(|l| serde_json::from_str::<Value>(l).is_ok()),
            "every log line is JSON: {log}"
        );
        assert!(log.contains("register sync finished"), "{log}");
        for secret in ADDRESSES
            .iter()
            .copied()
            .chain(["fau_register:", "127.0.0.1"])
        {
            assert!(!log.contains(secret), "{secret} reached the log: {log}");
        }
        assert!(out.stdout.is_empty(), "a real run prints nothing on stdout");
    }
}

#[tokio::test]
async fn a_dry_run_prints_the_plan_and_writes_only_its_run_row() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    let dry = fau(&["sync", "--seed", "--dry-run"], &env).await;
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    let printed = String::from_utf8(dry.stdout.clone()).unwrap();
    for address in ADDRESSES {
        assert!(!printed.contains(address), "{address} in the dry run");
    }
    let plan: Value = serde_json::from_str(&printed).expect("the dry run prints JSON");
    assert_eq!(
        (&plan["kind"], &plan["outcome"]),
        (&json!("seed"), &json!("apply"))
    );
    assert_eq!(plan["counts"]["municipalities_created"], json!(358));
    assert_eq!(plan["counts"]["schools_created"], json!(9));
    let hosle: Vec<&Value> = plan["school_ops"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| op["orgnr"] == json!("974552124"))
        .collect();
    assert_eq!(hosle.len(), 1);
    assert_eq!(
        (
            &hosle[0]["op"],
            &hosle[0]["slug"],
            &hosle[0]["verification"]
        ),
        (
            &json!("create_school"),
            &json!("hosle-skole"),
            &json!("listed")
        )
    );

    assert_eq!(
        runs(&admin).await,
        [("dry_run".into(), Some("no_change".into()), None)]
    );
    assert_eq!(count(&admin, "municipalities").await, 0);
    assert_eq!(count(&admin, "register_source_records").await, 0);
    assert_eq!(count(&admin, "outbox").await, 0);

    // A dry run leaves the register empty, so the seed itself may still run.
    let seed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(seed.status.code(), Some(0), "{}", stderr(&seed));
}

#[tokio::test]
async fn the_sync_refuses_an_empty_register_a_second_seed_and_a_held_lock() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    let unseeded = fau(&["sync"], &env).await;
    assert_eq!(unseeded.status.code(), Some(5), "{}", stderr(&unseeded));
    assert!(stderr(&unseeded).contains("the register is empty: seed it first with --seed"));
    assert!(runs(&admin).await.is_empty(), "a refusal writes no run row");

    let seed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(seed.status.code(), Some(0), "{}", stderr(&seed));
    let again = fau(&["sync", "--seed"], &env).await;
    assert_eq!(again.status.code(), Some(5), "{}", stderr(&again));
    assert!(stderr(&again).contains("--seed refuses a register that is not empty"));

    let pool = db.register_pool().await;
    let mut holder = pool.acquire().await.unwrap();
    assert!(try_lock(&mut holder).await.unwrap());
    let locked = fau(&["sync"], &env).await;
    assert_eq!(locked.status.code(), Some(4), "{}", stderr(&locked));
    assert!(stderr(&locked).contains("another run holds the lock"));

    assert_eq!(
        runs(&admin).await,
        [("seed".into(), Some("applied".into()), None)]
    );
}

#[tokio::test]
async fn an_empty_nsr_list_aborts_the_seed_and_mails_the_operator() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    sources.empty_nsr.store(true, Ordering::SeqCst);

    let out = fau(&["sync", "--seed"], &sources.env(&db.register_url())).await;
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(
        runs(&admin).await,
        [(
            "seed".into(),
            Some("aborted".into()),
            Some("empty_source:nsr".into())
        )]
    );
    assert_eq!(outbox_templates(&admin).await, ["register.sync_aborted"]);
    assert_eq!(count(&admin, "municipalities").await, 0);
}

#[tokio::test]
async fn the_runtime_role_cannot_run_the_sync() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;

    let out = fau(&["sync", "--seed"], &sources.env(&db.url())).await;
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("sqlstate 42501"), "{}", stderr(&out));
    assert!(runs(&admin).await.is_empty());
    assert_eq!(count(&admin, "municipalities").await, 0);
}

#[tokio::test]
async fn configuration_errors_name_the_variable_and_never_its_value() {
    let missing = run_fau_register(&["sync"], &[]).await;
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        stderr(&missing),
        "fau: configuration variable REGISTER_DATABASE_URL is missing\n"
    );

    let leaky = run_fau_register(
        &["sync"],
        &[
            ("REGISTER_DATABASE_URL", "postgres://u:p@127.0.0.1:1/fau"),
            (
                "REGISTER_NSR_URL",
                "https://user:banana-sentinel@data-nsr.udir.no",
            ),
        ],
    )
    .await;
    assert_eq!(leaky.status.code(), Some(1));
    assert_eq!(
        stderr(&leaky),
        "fau: configuration variable REGISTER_NSR_URL is not a valid value\n"
    );
}

/// Controller hand-off on Task 4's review: the advisory lock is taken on a dedicated
/// connection (`PgConnection::connect`), never a pooled one, so closing that connection on
/// any exit path -- including a source failure well after the lock was taken -- releases the
/// lock. A run that fails mid-fetch (NSR's detail endpoint answering 500) must not leave the
/// next run finding the lock still held.
#[tokio::test]
async fn a_run_that_fails_mid_fetch_releases_the_lock_for_the_next_one() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    sources.fail_detail.store(true, Ordering::SeqCst);
    let failed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(failed.status.code(), Some(1), "{}", stderr(&failed));
    assert_eq!(
        runs(&admin).await,
        [(
            "seed".into(),
            Some("failed".into()),
            Some("source_error".into())
        )],
        "a fixed code, never the source's error text"
    );
    assert_eq!(
        count(&admin, "municipalities").await,
        0,
        "nothing was ever applied: the failure was in the fetch phase"
    );

    sources.fail_detail.store(false, Ordering::SeqCst);
    let retried = fau(&["sync", "--seed"], &env).await;
    assert_eq!(
        retried.status.code(),
        Some(0),
        "the lock was released by the first run's connection closing: {}",
        stderr(&retried)
    );
    assert_eq!(count(&admin, "municipalities").await, 358);
    assert_eq!(count(&admin, "schools").await, 9);
}

/// Controller hand-off on Task 4's review: when the apply transaction itself fails -- here,
/// `record_applied`'s own audit-event insert, denied by revoking the privilege fau_register
/// otherwise has -- the run must be recorded as `failed` on a separate connection or
/// transaction *after* the rollback, with a fixed reason code, never the database error's
/// text. Proves both halves: the whole transaction (including the municipality/school rows
/// `apply_plan` already wrote) rolls back, and the run row is still written correctly
/// afterwards -- which only works if the rollback was awaited before `conn` was reused, not
/// left to `Transaction`'s drop glue.
#[tokio::test]
async fn an_apply_failure_rolls_back_and_records_a_fixed_reason() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    sqlx::query("revoke insert on audit_events from fau_register")
        .execute(&admin)
        .await
        .unwrap();

    let failed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(failed.status.code(), Some(1), "{}", stderr(&failed));
    assert_eq!(
        runs(&admin).await,
        [(
            "seed".into(),
            Some("failed".into()),
            Some("apply_error".into())
        )],
        "a fixed code, never the database error's own text"
    );
    assert!(
        !stderr(&failed).contains("audit_events"),
        "the table name must not reach the log: {}",
        stderr(&failed)
    );
    assert_eq!(
        count(&admin, "municipalities").await,
        0,
        "apply_plan's own writes must have rolled back with the rest of the transaction"
    );
    assert_eq!(count(&admin, "schools").await, 0);
    assert_eq!(count(&admin, "outbox").await, 0);

    sqlx::query("grant insert on audit_events to fau_register")
        .execute(&admin)
        .await
        .unwrap();
    let retried = fau(&["sync", "--seed"], &env).await;
    assert_eq!(
        retried.status.code(),
        Some(0),
        "the failed run's connection released the lock, and the register is still empty: {}",
        stderr(&retried)
    );
    assert_eq!(count(&admin, "municipalities").await, 358);
    assert_eq!(count(&admin, "schools").await, 9);
}
