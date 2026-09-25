//! `fau register sync` end to end (#3441 part 4): the real binary, as `fau_register`,
//! against a real test database, with every source URL pointed at an in-process server
//! that serves the recorded fixtures in crates/register-sources/tests/fixtures/. No test
//! calls the network.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use common::register::{listed_school, run_fau_register, spawn_fau_register};
use common::TestDb;
use fau_persistence::register::try_lock;
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::sync::Notify;
use uuid::Uuid;

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

fn rename_if_hosle(unit: &mut Value) {
    if unit["Organisasjonsnummer"] == json!("974552124") {
        unit["Navn"] = json!("Hosle barneskole");
    }
}

/// The in-process sources. `empty_nsr` makes NSR answer with an empty list; `fail_detail`
/// makes every NSR detail request answer 500 (controller hand-off: a mid-run source failure
/// must still release the advisory lock); `ssb_queries` records every SSB `(from, to)`.
/// `pause_kartverket`/`paused`/`resume` let a test freeze the run right after it has fetched
/// nothing yet but already inserted its run row and taken the lock (Kartverket is the first
/// thing a seed fetches), so it can break the run's own database connection from outside at a
/// known point rather than racing it.
struct Sources {
    base: String,
    empty_nsr: Arc<AtomicBool>,
    fail_detail: Arc<AtomicBool>,
    ssb_queries: Arc<Mutex<Vec<(String, String)>>>,
    pause_kartverket: Arc<AtomicBool>,
    paused: Arc<Notify>,
    resume: Arc<Notify>,
    /// Milliseconds Kartverket waits before it answers: a run that visibly takes time.
    slow_kartverket_ms: Arc<AtomicU64>,
    /// Orgnrs whose next detail request answers 503, once each.
    flaky_detail: Arc<Mutex<HashSet<String>>>,
    /// Every NSR detail request, by orgnr.
    detail_hits: Arc<Mutex<Vec<String>>>,
    /// NSR calls Hosle skole "Hosle barneskole", in the list and the detail: one rename of
    /// nine schools, which trips the 5% rename breaker.
    rename_hosle: Arc<AtomicBool>,
}

impl Sources {
    async fn start() -> Self {
        let empty_nsr = Arc::new(AtomicBool::new(false));
        let fail_detail = Arc::new(AtomicBool::new(false));
        let ssb_queries = Arc::new(Mutex::new(Vec::new()));
        let pause_kartverket = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let slow_kartverket_ms = Arc::new(AtomicU64::new(0));
        let slow = slow_kartverket_ms.clone();
        let flaky_detail = Arc::new(Mutex::new(HashSet::new()));
        let detail_hits = Arc::new(Mutex::new(Vec::new()));
        let (flaky, hits) = (flaky_detail.clone(), detail_hits.clone());
        let rename_hosle = Arc::new(AtomicBool::new(false));
        let (rename_list, rename_detail) = (rename_hosle.clone(), rename_hosle.clone());
        let (empty, queries) = (empty_nsr.clone(), ssb_queries.clone());
        let fail = fail_detail.clone();
        let (pause, entered, released) = (pause_kartverket.clone(), paused.clone(), resume.clone());
        let app = Router::new()
            .route(
                "/nsr/v4/enheter",
                get(move || {
                    let empty = empty.load(Ordering::SeqCst);
                    let rename = rename_list.load(Ordering::SeqCst);
                    async move {
                        let body = if empty {
                            json!({
                                "Sidenummer": 1, "AntallPerSide": 1000, "AntallSider": 1,
                                "TotaltAntallEnheter": 0, "EnhetListe": [],
                            })
                        } else {
                            let mut list = nsr_list();
                            if rename {
                                for unit in list["EnhetListe"].as_array_mut().unwrap() {
                                    rename_if_hosle(unit);
                                }
                            }
                            list
                        };
                        serde_json::to_vec(&body).unwrap()
                    }
                }),
            )
            .route(
                "/nsr/v4/enhet/{orgnr}",
                get(move |Path(orgnr): Path<String>| {
                    let fail = fail.load(Ordering::SeqCst);
                    hits.lock().unwrap().push(orgnr.clone());
                    let flaky = flaky.lock().unwrap().remove(&orgnr);
                    let rename = rename_detail.load(Ordering::SeqCst);
                    async move {
                        if fail {
                            return (StatusCode::INTERNAL_SERVER_ERROR, Vec::new());
                        }
                        if flaky {
                            return (StatusCode::SERVICE_UNAVAILABLE, Vec::new());
                        }
                        match std::fs::read(format!("{FIXTURES}/nsr/enhet-{orgnr}.json")) {
                            Ok(body) if rename => {
                                let mut unit: Value = serde_json::from_slice(&body).unwrap();
                                rename_if_hosle(&mut unit);
                                (StatusCode::OK, serde_json::to_vec(&unit).unwrap())
                            }
                            Ok(body) => (StatusCode::OK, body),
                            Err(_) => (StatusCode::NOT_FOUND, Vec::new()),
                        }
                    }
                }),
            )
            .route(
                "/kv/fylkerkommuner",
                get(move || {
                    let (pause, entered, released) =
                        (pause.clone(), entered.clone(), released.clone());
                    let slow = slow.load(Ordering::SeqCst);
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(slow)).await;
                        if pause.load(Ordering::SeqCst) {
                            entered.notify_one();
                            released.notified().await;
                        }
                        fixture("kartverket/fylkerkommuner.json")
                    }
                }),
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
            pause_kartverket,
            paused,
            resume,
            slow_kartverket_ms,
            flaky_detail,
            detail_hits,
            rename_hosle,
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
            // Three attempts, as in production, but without its 1 s and 4 s waits.
            ("REGISTER_SOURCE_RETRY_DELAYS_MS", "10,20".to_owned()),
        ]
    }
}

async fn fau(args: &[&str], env: &[(&'static str, String)]) -> std::process::Output {
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    run_fau_register(args, &env).await
}

/// As [`fau`], but spawns without waiting -- see [`spawn_fau_register`].
fn fau_spawn(args: &[&str], env: &[(&'static str, String)]) -> tokio::process::Child {
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    spawn_fau_register(args, &env)
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
/// connection (`PgConnection::connect`), never a pooled one. This proves the consequence, not
/// anything the code does explicitly to release the lock: a run that fails mid-fetch (NSR's
/// detail endpoint answering 500) exits, its process-local connection drops, and *that* --
/// the connection closing, which running to completion (however it ends) always does -- is
/// what releases the lock, so the next run does not find it still held.
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
/// otherwise has -- the run must be recorded as `failed` with a fixed reason code, never the
/// database error's text, after the transaction is rolled back. Proves both halves: the whole
/// transaction (including the municipality/school rows `apply_plan` already wrote) rolls
/// back, and the run row is still written correctly afterwards -- which only works if the
/// rollback was awaited before the connection used to record the failure was opened, not left
/// to `Transaction`'s drop glue. The retried seed afterwards proves the lock is gone too, but
/// that is the process exiting and its connection closing, same as any other run -- nothing
/// this test does releases it explicitly.
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
        "the failed run's process exited and its connection closed, so the lock is gone and \
         the register is still empty: {}",
        stderr(&retried)
    );
    assert_eq!(count(&admin, "municipalities").await, 358);
    assert_eq!(count(&admin, "schools").await, 9);
}

/// Controller hand-off, fix round 1: when the run's own database connection breaks -- an io
/// error, or its backend killed, e.g. by a Postgres restart -- recording it as `failed` must
/// happen on a *fresh* connection, never the broken one, or the write fails too and the run
/// row is left `started_at` set, `finished_at` null, forever (the CronJob's and #3442's
/// alerting would never see it as done).
///
/// Paused right after the run has taken the lock and inserted its row (Kartverket is the
/// first thing a seed fetches, so pausing there lands after `begin()` but before any other
/// database work), so the backend can be killed from the admin pool at a known point instead
/// of racing it. The kill lands before the run's next database operation (opening the apply
/// transaction), which is what actually surfaces the break -- not literally inside
/// `apply_plan` itself, but the same code path handles a break at either point identically.
#[tokio::test]
async fn a_connection_broken_after_the_run_row_exists_still_finishes_it_as_failed() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    sources.pause_kartverket.store(true, Ordering::SeqCst);
    let child = fau_spawn(&["sync", "--seed"], &env);
    sources.paused.notified().await;

    // `pg_stat_activity` is cluster-wide, not scoped to this test's own database, so every
    // condition here matters: without `datname`, this would terminate another concurrently
    // running test's `fau_register` connection too.
    sqlx::query(
        "select pg_terminate_backend(pid) from pg_stat_activity
          where usename = 'fau_register' and datname = $1 and pid <> pg_backend_pid()",
    )
    .bind(&db.name)
    .execute(&admin)
    .await
    .unwrap();
    sources.resume.notify_one();

    let out = tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
        .await
        .expect("fau register did not exit within 30s after its connection was killed")
        .expect("wait for fau register");
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{log}");
    assert_eq!(
        runs(&admin).await,
        [(
            "seed".into(),
            Some("failed".into()),
            Some("database_error".into())
        )],
        "the run row must still be finished -- on a fresh connection, since the one the run \
         held is dead -- never left unfinished forever"
    );
    assert_eq!(count(&admin, "municipalities").await, 0);

    // The dead connection's session, and the lock with it, is gone the moment Postgres
    // notices the backend killed -- nothing this test does releases it explicitly. The pause
    // is disarmed first, or the retried run's own Kartverket fetch would block forever on the
    // same gate with nothing left to resume it.
    sources.pause_kartverket.store(false, Ordering::SeqCst);
    let retried = fau(&["sync", "--seed"], &env).await;
    assert_eq!(retried.status.code(), Some(0), "{}", stderr(&retried));
    assert_eq!(count(&admin, "municipalities").await, 358);
    assert_eq!(count(&admin, "schools").await, 9);
}

/// Controller hand-off, fix round 1: `record_aborted` writes the run row and its
/// `register.sync_aborted` mail as two statements, wrapped in one transaction so they commit
/// together. Revoking `insert` on `outbox` from `fau_register` makes the second statement
/// fail; the whole abort-recording transaction must then roll back rather than leave a run
/// row with no mail, and the run must still end as `failed` with a fixed reason, on the
/// pattern of `an_apply_failure_rolls_back_and_records_a_fixed_reason`.
#[tokio::test]
async fn an_abort_recording_failure_rolls_back_and_still_records_failed() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    sources.empty_nsr.store(true, Ordering::SeqCst);
    let env = sources.env(&db.register_url());

    sqlx::query("revoke insert on outbox from fau_register")
        .execute(&admin)
        .await
        .unwrap();

    let out = fau(&["sync", "--seed"], &env).await;
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(
        runs(&admin).await,
        [(
            "seed".into(),
            Some("failed".into()),
            Some("database_error".into())
        )],
        "a fixed code, never the database error's own text"
    );
    assert_eq!(
        count(&admin, "outbox").await,
        0,
        "the run row and its mail must commit together: neither one alone"
    );
    assert_eq!(count(&admin, "municipalities").await, 0);
}

/// Pins the exit code for a sync with no applied seed run to start its SSB lookback from --
/// a register made non-empty directly (bypassing `--seed` entirely), so `register_is_empty`
/// lets a plain `sync` proceed but `seed_date` finds no applied `seed` row.
#[tokio::test]
async fn a_sync_with_no_applied_seed_exits_1_with_a_fixed_reason() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    listed_school(&admin, Uuid::now_v7(), "Direkte Skole").await;
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    let out = fau(&["sync"], &env).await;
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(
        runs(&admin).await,
        [("sync".into(), Some("failed".into()), Some("no_seed".into()))]
    );
}

/// Pins the exit code for a dry run whose plan would abort: `record_dry_run` always records
/// `no_change` (a dry run changes nothing), carrying the would-be abort reason, but the
/// process itself must still exit 3, the same code a real aborted run exits with.
#[tokio::test]
async fn a_dry_run_that_would_abort_exits_3() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    sources.empty_nsr.store(true, Ordering::SeqCst);

    let out = fau(
        &["sync", "--seed", "--dry-run"],
        &sources.env(&db.register_url()),
    )
    .await;
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(
        runs(&admin).await,
        [(
            "dry_run".into(),
            Some("no_change".into()),
            Some("empty_source:nsr".into())
        )]
    );
    assert_eq!(count(&admin, "municipalities").await, 0);
}

/// Final review, item 1: a run row's `finished_at`, and its audit entry's `occurred_at`, are
/// when the run actually finished -- never the `Moment` it started at.
#[tokio::test]
async fn a_run_records_when_it_actually_finished() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    sources.slow_kartverket_ms.store(400, Ordering::SeqCst);

    let out = fau(&["sync", "--seed"], &sources.env(&db.register_url())).await;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let (run_ms, audit_ms, mail_ms): (f64, f64, f64) = sqlx::query_as(
        "select extract(epoch from r.finished_at - r.started_at)::float8 * 1000,
                extract(epoch from a.occurred_at - r.started_at)::float8 * 1000,
                extract(epoch from o.created_at - r.started_at)::float8 * 1000
           from register_sync_runs r
           join audit_events a on a.subject_id = r.id
           join outbox o on o.template = 'register.seed_summary'",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    for (what, ms) in [
        ("finished_at", run_ms),
        ("occurred_at", audit_ms),
        ("mail", mail_ms),
    ] {
        assert!(ms >= 400.0, "{what} is only {ms} ms after started_at");
    }

    // A failed run, too: its finish time is taken when the failure is recorded.
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    sources.fail_detail.store(true, Ordering::SeqCst);
    let failed = fau(&["sync", "--seed"], &sources.env(&db.register_url())).await;
    assert_eq!(failed.status.code(), Some(1), "{}", stderr(&failed));
    let failed_ms: f64 = sqlx::query_scalar(
        "select extract(epoch from finished_at - started_at)::float8 * 1000
           from register_sync_runs",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert!(
        failed_ms >= 400.0,
        "a failed run finished {failed_ms} ms after it started"
    );
}

/// The log lines of `out` whose message is `message`, parsed.
fn log_lines(out: &std::process::Output, message: &str) -> Vec<Value> {
    stderr(out)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|l| l["message"] == json!(message))
        .collect()
}

/// Final review, item 2: a detail request that answers 503 once is retried, and the run
/// succeeds. The retry is logged with the source and the kind, never the URL.
#[tokio::test]
async fn a_detail_that_fails_once_with_503_is_retried_and_the_run_succeeds() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    sources
        .flaky_detail
        .lock()
        .unwrap()
        .insert("974552124".to_owned());

    let out = fau(&["sync", "--seed"], &sources.env(&db.register_url())).await;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(count(&admin, "schools").await, 9);
    let hosle_hits = sources
        .detail_hits
        .lock()
        .unwrap()
        .iter()
        .filter(|o| *o == "974552124")
        .count();
    assert_eq!(hosle_hits, 2, "one 503, then one success");
    let retries = log_lines(&out, "source request failed, retrying");
    assert_eq!(retries.len(), 1, "{}", stderr(&out));
    let log = stderr(&out);
    assert!(log.contains("Status"), "{log}");
    assert!(!log.contains("127.0.0.1") && !log.contains("/v4/"), "{log}");
}

/// Final review, item 2: a detail fetch that keeps failing names the failing orgnr and the
/// error's source and kind in its log line, after three attempts, and never a URL or a body.
#[tokio::test]
async fn a_persistent_detail_failure_is_logged_with_its_orgnr_and_kind() {
    let db = TestDb::migrated().await;
    let sources = Sources::start().await;
    sources.fail_detail.store(true, Ordering::SeqCst);

    let out = fau(&["sync", "--seed"], &sources.env(&db.register_url())).await;
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let failures = log_lines(&out, "NSR detail fetch failed");
    assert!(!failures.is_empty(), "{}", stderr(&out));
    for line in &failures {
        let fields = line;
        let orgnr = fields["orgnr"].as_str().expect("the orgnr is logged");
        assert!(ORGNRS.contains(&orgnr), "{line}");
        assert_eq!(fields["source"], json!("Nsr"), "{line}");
        assert_eq!(fields["kind"], json!("Status { status: 500 }"), "{line}");
    }
    let hits = sources.detail_hits.lock().unwrap().clone();
    let first = failures[0]["orgnr"].as_str().unwrap();
    assert_eq!(
        hits.iter().filter(|o| *o == first).count(),
        3,
        "three attempts in total: {hits:?}"
    );
    let log = stderr(&out);
    assert!(!log.contains("127.0.0.1") && !log.contains("/v4/"), "{log}");
}

/// Final review, item 11: the detail pass reports its progress in counts only.
#[tokio::test]
async fn the_detail_pass_logs_its_progress_in_counts() {
    let db = TestDb::migrated().await;
    let sources = Sources::start().await;
    let out = fau(&["sync", "--seed"], &sources.env(&db.register_url())).await;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let done = log_lines(&out, "NSR detail pass finished");
    assert_eq!(done.len(), 1, "{}", stderr(&out));
    assert_eq!(
        (&done[0]["fetched"], &done[0]["total"]),
        (&json!(12), &json!(12))
    );
}

/// Final review, item 3: a sync blocked by a lock another session holds gives up after the
/// connection's 10 s `lock_timeout`, exits 1 and records `database_timeout`, rather than
/// hanging the CronJob until its deadline.
#[tokio::test]
async fn a_sync_blocked_on_a_lock_fails_with_database_timeout() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());

    // Only the snapshot reads this table: the lock, the emptiness check and the run row all
    // get through, and the run blocks once it has started.
    let mut holder = admin.begin().await.unwrap();
    sqlx::query("lock table school_slug_history in access exclusive mode")
        .execute(&mut *holder)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        fau(&["sync", "--seed"], &env),
    )
    .await
    .expect("the blocked sync must give up on its own, well before 45 s");
    let waited = started.elapsed();
    holder.rollback().await.unwrap();

    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        waited >= std::time::Duration::from_secs(9),
        "it waited the lock timeout out: {waited:?}"
    );
    assert_eq!(
        runs(&admin).await,
        [(
            "seed".into(),
            Some("failed".into()),
            Some("database_timeout".into())
        )]
    );
}

/// Final review, item 4: a Sync the breaker aborts stays aborted without the flag; its dry
/// run prints every op and review so the operator can see what tripped it, and still exits 3;
/// with `--accept-mass-change` the same plan applies, and the run's audit entry says so.
#[tokio::test]
async fn accept_mass_change_applies_what_the_breaker_stopped() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    let sources = Sources::start().await;
    let env = sources.env(&db.register_url());
    let seed = fau(&["sync", "--seed"], &env).await;
    assert_eq!(seed.status.code(), Some(0), "{}", stderr(&seed));
    sources.rename_hosle.store(true, Ordering::SeqCst);

    let aborted = fau(&["sync"], &env).await;
    assert_eq!(aborted.status.code(), Some(3), "{}", stderr(&aborted));

    let dry = fau(&["sync", "--dry-run"], &env).await;
    assert_eq!(dry.status.code(), Some(3), "{}", stderr(&dry));
    let printed: Value = serde_json::from_slice(&dry.stdout).expect("the dry run prints JSON");
    assert_eq!(printed["outcome"], json!("abort"));
    assert_eq!(
        printed["abort_reason"],
        json!("mass_change:closes=0,renames=1,attribute_losses=0,active=9")
    );
    let renames: Vec<&Value> = printed["school_ops"]
        .as_array()
        .expect("an aborted dry run lists its ops")
        .iter()
        .filter(|op| op["op"] == json!("rename_school"))
        .collect();
    assert_eq!(renames.len(), 1, "{printed}");
    assert_eq!(renames[0]["register_name"], json!("Hosle barneskole"));
    assert!(printed["reviews"].is_array(), "{printed}");
    assert!(printed["municipality_ops"].is_array(), "{printed}");

    let accepted = fau(&["sync", "--accept-mass-change"], &env).await;
    assert_eq!(accepted.status.code(), Some(0), "{}", stderr(&accepted));
    assert!(
        stderr(&accepted).contains("mass change accepted"),
        "{}",
        stderr(&accepted)
    );
    assert_eq!(
        runs(&admin).await,
        [
            ("seed".into(), Some("applied".into()), None),
            (
                "sync".into(),
                Some("aborted".into()),
                Some("mass_change:closes=0,renames=1,attribute_losses=0,active=9".into())
            ),
            (
                "dry_run".into(),
                Some("no_change".into()),
                Some("mass_change:closes=0,renames=1,attribute_losses=0,active=9".into())
            ),
            ("sync".into(), Some("applied".into()), None),
        ]
    );
    let params: Value = sqlx::query_scalar(
        "select a.params from audit_events a
           join register_sync_runs r on r.id = a.subject_id
          where r.kind = 'sync'",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(params["mass_change_accepted"], json!(true), "{params}");
    assert_eq!(
        params["mass_change"],
        json!("mass_change:closes=0,renames=1,attribute_losses=0,active=9")
    );
    let name: String =
        sqlx::query_scalar("select register_name from schools where orgnr = '974552124'")
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(name, "Hosle barneskole");
}

/// The override is never honoured on a seed, which has no breaker: clap refuses the pair.
#[tokio::test]
async fn accept_mass_change_is_refused_on_a_seed() {
    let db = TestDb::migrated().await;
    let sources = Sources::start().await;
    let out = fau(
        &["sync", "--seed", "--accept-mass-change"],
        &sources.env(&db.register_url()),
    )
    .await;
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("cannot be used with"),
        "a conflict, not an unknown flag: {}",
        stderr(&out)
    );
    assert!(runs(&db.admin_pool()).await.is_empty());
}
