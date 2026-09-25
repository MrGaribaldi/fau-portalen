//! Register run bookkeeping (#3441 part 4): the advisory lock, the run row, the audit entry
//! and the operator's mail, as `fau_register`.

mod common;

use common::TestDb;
use fau_domain::register::sync::testkit::{self, fixture_records, hosle, lerberg};
use fau_domain::register::sync::{plan, AbortReason, Counts, RunKind, SyncOutcome, SyncPlan};
use fau_domain::time::Moment;
use fau_persistence::register::{
    apply_plan, load_snapshot, record_aborted, record_applied, record_dry_run, record_failed,
    record_no_change, seed_date, start_run, try_lock, unlock, RegisterError,
};
use jiff::civil::date;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn at(rfc3339: &str) -> Moment {
    Moment::at(rfc3339.parse().expect("an RFC 3339 timestamp"))
}

fn applied(outcome: SyncOutcome<Uuid>) -> SyncPlan<Uuid> {
    match outcome {
        SyncOutcome::Apply(plan) => plan,
        other => panic!("expected a plan, got {other:?}"),
    }
}

/// Plans `units` against the database's register and applies the plan as a run of `kind`
/// at `at`, recording it: start, apply, record, commit, as `fau register sync` does.
async fn run(
    pool: &PgPool,
    units: &[fau_domain::register::source::NsrUnit],
    kind: RunKind,
    at: Moment,
) -> Uuid {
    let mut conn = pool.acquire().await.unwrap();
    let id = start_run(&mut conn, kind, false, at).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let snapshot = load_snapshot(&mut tx).await.unwrap();
    let plan = applied(plan(
        &snapshot,
        &testkit::inputs(&fixture_records(), &[], units, kind),
    ));
    let counts = apply_plan(&mut tx, &plan, at).await.unwrap();
    record_applied(&mut tx, id, kind, &counts, at.now())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

async fn run_row(admin: &PgPool, id: Uuid) -> (String, Option<String>, Value, Option<String>) {
    sqlx::query_as(
        "select kind, outcome, counts, abort_reason from register_sync_runs where id = $1",
    )
    .bind(id)
    .fetch_one(admin)
    .await
    .unwrap()
}

async fn outbox(admin: &PgPool) -> Vec<(Option<Uuid>, String, String, Value)> {
    sqlx::query_as(
        "select tenant_id, template, recipient_email, params from outbox order by created_at, id",
    )
    .fetch_all(admin)
    .await
    .unwrap()
}

fn counts(pairs: &[(&str, u64)]) -> Value {
    let mut v = json!({
        "municipalities_created": 0, "municipalities_updated": 0, "renumbered": 0,
        "renamed": 0, "schools_created": 0, "schools_renamed": 0, "schools_updated": 0,
        "attribute_losses": 0, "schools_moved": 0, "schools_closed": 0, "schools_held": 0,
        "reviews": 0, "skipped": 0,
    });
    for (k, n) in pairs {
        v[*k] = json!(n);
    }
    v
}

#[tokio::test]
async fn one_session_holds_the_lock_until_it_lets_go() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let mut first = pool.acquire().await.unwrap();
    let mut second = pool.acquire().await.unwrap();
    assert!(try_lock(&mut first).await.unwrap());
    assert!(!try_lock(&mut second).await.unwrap(), "held by the first");
    unlock(&mut first).await.unwrap();
    assert!(try_lock(&mut second).await.unwrap());
}

#[tokio::test]
async fn an_applied_seed_records_its_run_one_audit_entry_and_one_summary_mail() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let id = run(&pool, &[hosle()], RunKind::Seed, at("2026-09-28T02:30:00Z")).await;

    let mut expected = counts(&[("municipalities_created", 9), ("schools_created", 1)]);
    expected["reviews_written"] = json!(0);
    expected["reviews_deduplicated"] = json!(0);
    assert_eq!(
        run_row(&admin, id).await,
        (
            "seed".into(),
            Some("applied".into()),
            expected.clone(),
            None
        )
    );

    let audit: Vec<(Option<Uuid>, String, String, String, Uuid, Value)> = sqlx::query_as(
        "select tenant_id, actor_kind, action, subject_type, subject_id, params from audit_events",
    )
    .fetch_all(&admin)
    .await
    .unwrap();
    let mut params = expected.clone();
    params["kind"] = json!("seed");
    assert_eq!(
        audit,
        [(
            None,
            "system".into(),
            "register.sync_applied".into(),
            "register_sync_run".into(),
            id,
            params
        )]
    );

    assert_eq!(
        outbox(&admin).await,
        [(
            None,
            "register.seed_summary".into(),
            "fau@ewb-solutions.as".into(),
            json!({ "run_id": id, "counts": expected })
        )]
    );
}

#[tokio::test]
async fn an_applied_sync_mails_each_review_item_it_wrote() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    run(
        &pool,
        &[hosle(), lerberg()],
        RunKind::Seed,
        at("2026-09-28T02:30:00Z"),
    )
    .await;
    sqlx::query("delete from outbox")
        .execute(&admin)
        .await
        .unwrap();
    // Two units under numbers nobody knows: two review items.
    let units = [
        hosle(),
        lerberg(),
        testkit::unit("974000002", "Nordpolen skole", "9999"),
        testkit::unit("974000003", "Sydpolen skole", "9998"),
    ];
    let id = run(&pool, &units, RunKind::Sync, at("2026-10-05T02:30:00Z")).await;

    let items: Vec<(Uuid, String)> =
        sqlx::query_as("select id, kind from register_review_items order by id")
            .fetch_all(&admin)
            .await
            .unwrap();
    assert_eq!(items.len(), 2);
    let mails = outbox(&admin).await;
    assert_eq!(
        mails,
        items
            .iter()
            .map(|(item, kind)| (
                None,
                "register.review_item".to_owned(),
                "fau@ewb-solutions.as".to_owned(),
                json!({ "run_id": id, "review_item_id": item, "kind": kind })
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        run_row(&admin, id).await.2["reviews_written"],
        json!(2),
        "the run records what it wrote"
    );
}

#[tokio::test]
async fn an_abort_records_its_reason_and_one_mail_and_nothing_else() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let mut conn = pool.acquire().await.unwrap();
    let now = at("2026-10-05T02:30:00Z");

    let empty = start_run(&mut conn, RunKind::Seed, false, now)
        .await
        .unwrap();
    record_aborted(
        &mut conn,
        empty,
        RunKind::Seed,
        &AbortReason::EmptySource { source: "nsr" },
        &Counts::default(),
        now.now(),
    )
    .await
    .unwrap();
    let mass = start_run(&mut conn, RunKind::Sync, false, now)
        .await
        .unwrap();
    let mass_counts = Counts {
        schools_closed: 3,
        ..Counts::default()
    };
    record_aborted(
        &mut conn,
        mass,
        RunKind::Sync,
        &AbortReason::MassChange {
            closes: 3,
            renames: 0,
            attribute_losses: 0,
            active: 100,
        },
        &mass_counts,
        now.now(),
    )
    .await
    .unwrap();

    assert_eq!(
        run_row(&admin, empty).await,
        (
            "seed".into(),
            Some("aborted".into()),
            counts(&[]),
            Some("empty_source:nsr".into())
        )
    );
    assert_eq!(
        run_row(&admin, mass).await,
        (
            "sync".into(),
            Some("aborted".into()),
            counts(&[("schools_closed", 3)]),
            Some("mass_change:closes=3,renames=0,attribute_losses=0,active=100".into())
        )
    );
    let to = || "fau@ewb-solutions.as".to_owned();
    assert_eq!(
        outbox(&admin).await,
        [
            (
                None,
                "register.sync_aborted".to_owned(),
                to(),
                json!({ "run_id": empty, "kind": "seed", "reason": "empty_source", "source": "nsr" })
            ),
            (
                None,
                "register.sync_aborted".to_owned(),
                to(),
                json!({
                    "run_id": mass, "kind": "sync", "reason": "mass_change",
                    "closes": 3, "renames": 0, "attribute_losses": 0, "active": 100
                })
            ),
        ]
    );
    let audited: i64 = sqlx::query_scalar("select count(*) from audit_events")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(audited, 0, "only an applied run is audited");
}

#[tokio::test]
async fn no_change_dry_run_and_failed_rows_finish_once() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let mut conn = pool.acquire().await.unwrap();
    let now = at("2026-10-05T02:30:00Z");

    let quiet = start_run(&mut conn, RunKind::Sync, false, now)
        .await
        .unwrap();
    record_no_change(&mut conn, quiet, now.now()).await.unwrap();
    let dry = start_run(&mut conn, RunKind::Sync, true, now)
        .await
        .unwrap();
    record_dry_run(
        &mut conn,
        dry,
        &Counts {
            schools_created: 2,
            ..Counts::default()
        },
        None,
        now.now(),
    )
    .await
    .unwrap();
    let failed = start_run(&mut conn, RunKind::Sync, false, now)
        .await
        .unwrap();
    record_failed(&mut conn, failed, "source Nsr: Transport", now.now())
        .await
        .unwrap();

    assert_eq!(
        run_row(&admin, quiet).await,
        ("sync".into(), Some("no_change".into()), counts(&[]), None)
    );
    assert_eq!(
        run_row(&admin, dry).await,
        (
            "dry_run".into(),
            Some("no_change".into()),
            counts(&[("schools_created", 2)]),
            None
        )
    );
    assert_eq!(
        run_row(&admin, failed).await,
        (
            "sync".into(),
            Some("failed".into()),
            json!({}),
            Some("source Nsr: Transport".into())
        )
    );
    assert_eq!(
        record_no_change(&mut conn, quiet, now.now()).await,
        Err(RegisterError::UnknownRow),
        "a finished run is never finished again"
    );
    assert!(outbox(&admin).await.is_empty());
}

#[tokio::test]
async fn the_ssb_lookback_starts_on_the_oslo_date_of_the_first_applied_seed() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(seed_date(&mut conn).await.unwrap(), None);

    // A dry-run seed and an aborted seed do not count.
    let now = at("2026-09-20T12:00:00Z");
    let dry = start_run(&mut conn, RunKind::Seed, true, now)
        .await
        .unwrap();
    record_dry_run(&mut conn, dry, &Counts::default(), None, now.now())
        .await
        .unwrap();
    let aborted = start_run(&mut conn, RunKind::Seed, false, now)
        .await
        .unwrap();
    record_aborted(
        &mut conn,
        aborted,
        RunKind::Seed,
        &AbortReason::EmptySource { source: "nsr" },
        &Counts::default(),
        now.now(),
    )
    .await
    .unwrap();
    assert_eq!(seed_date(&mut conn).await.unwrap(), None);

    // 23:30 UTC on 27 September is already the 28th in Oslo.
    run(&pool, &[hosle()], RunKind::Seed, at("2026-09-27T23:30:00Z")).await;
    assert_eq!(seed_date(&mut conn).await.unwrap(), Some(date(2026, 9, 28)));
}

#[tokio::test]
async fn the_runtime_role_cannot_record_a_run() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        start_run(&mut conn, RunKind::Sync, false, at("2026-10-05T02:30:00Z")).await,
        Err(RegisterError::Database("sqlstate 42501".into()))
    );
}
