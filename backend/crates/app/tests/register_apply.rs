//! The register's persistence (#3441 part 4): the snapshot loader and the SQL applier,
//! against real PostgreSQL as `fau_register`, checked for parity with the planner's in-memory
//! test applier (`fau_domain::register::sync::testkit::apply`).

mod common;

use std::collections::{BTreeSet, HashMap};

use common::TestDb;
use fau_domain::register::source::{CodeChange, MunicipalityRecord, NsrUnit, OfficialName};
use fau_domain::register::sync::testkit::{self, change, fixture_records, gran_canaria, record};
use fau_domain::register::sync::{
    plan, MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, Origin, Ownership,
    RegisterSnapshot, RowId, RunKind, SchoolAttributes, SchoolSnapshot, SchoolStatus, SyncOutcome,
    SyncPlan, Verification,
};
use fau_domain::time::Moment;
use fau_persistence::register::{apply_plan, load_snapshot, register_is_empty, AppliedCounts};
use jiff::civil::date;
use sqlx::PgPool;
use uuid::Uuid;

const BAERUM: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_3201);
const OLD_ASKER: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0220);
const HOSLE: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0001_0001);
const SUBMITTED: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0001_0002);

async fn exec(admin: &PgPool, sql: &str) {
    sqlx::raw_sql(sql)
        .execute(admin)
        .await
        .unwrap_or_else(|e| panic!("fixture SQL failed: {e}"));
}

/// Every column the snapshot reads, with an empty string and a null side by side, an old
/// number next to the current one, a dissolved municipality, a live and a closed tenant, and
/// one row of each history table.
async fn fixture(admin: &PgPool) {
    exec(
        admin,
        "insert into municipalities (id, name, official_name, county_number, county_name, slug,
                                     status, dissolved_on, source, search_text)
         values ('01990000-0000-7000-8000-000000003201', 'Bærum', 'Bærum', '32', 'Akershus',
                 '3201-baerum', 'active', null, 'kartverket', 'bærum baerum barum'),
                ('01990000-0000-7000-8000-000000000220', 'Asker', null, '02', 'Akershus',
                 '0220-asker', 'dissolved', '2020-01-01', 'kartverket', 'asker');
         insert into municipality_names (municipality_id, name, language, priority)
         values ('01990000-0000-7000-8000-000000003201', 'Bærum', 'no', 1);
         insert into municipality_numbers (municipality_id, number, valid_from, valid_until)
         values ('01990000-0000-7000-8000-000000003201', '3024', '2020-01-01', '2024-01-01'),
                ('01990000-0000-7000-8000-000000003201', '3201', '2024-01-01', null),
                ('01990000-0000-7000-8000-000000000220', '0220', '1838-01-01', '2020-01-01');
         insert into municipality_slug_history (slug, municipality_id, valid_from, valid_until)
         values ('3024-baerum', '01990000-0000-7000-8000-000000003201',
                 '2020-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
         insert into schools (id, municipality_id, origin, display_name, register_name,
                              display_name_curated, slug, verification, orgnr, ownership,
                              grade_from, grade_to, register_language, website, street_address,
                              postcode, post_town, in_scope, scope_override, status, search_text)
         values ('01990000-0000-7000-8000-000000010001', '01990000-0000-7000-8000-000000003201',
                 'register', 'Hosle', 'Hosle skole', true, 'hosle', 'listed', '974552124',
                 'public', 1, 7, 'nb', '', 'Bispeveien 73', '1362', null, true, false, 'active',
                 'hosle'),
                ('01990000-0000-7000-8000-000000010002', '01990000-0000-7000-8000-000000003201',
                 'submitted', 'Nyskolen', null, false, null, 'pending', null, null,
                 null, null, null, null, null, null, null, true, null, 'active', 'nyskolen');
         insert into school_slug_history (municipality_id, slug, school_id, valid_from, valid_until)
         values ('01990000-0000-7000-8000-000000003201', 'hosle-skole',
                 '01990000-0000-7000-8000-000000010001',
                 '2024-01-01T00:00:00Z', '2025-01-01T00:00:00Z');
         insert into school_orgnr_history (orgnr, school_id, valid_from, valid_until)
         values ('975270920', '01990000-0000-7000-8000-000000010001',
                 '2020-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
         insert into tenants (id, name, status, school_id)
         values ('01990000-0000-7000-8000-000000020001', 'Hosle FAU', 'pending',
                 '01990000-0000-7000-8000-000000010001'),
                ('01990000-0000-7000-8000-000000020002', 'Gammelt FAU', 'closed',
                 '01990000-0000-7000-8000-000000010002');",
    )
    .await;
}

#[tokio::test]
async fn the_snapshot_reads_every_column_back_exactly() {
    let db = TestDb::migrated().await;
    fixture(&db.admin_pool()).await;
    let pool = db.register_pool().await;
    let mut conn = pool.acquire().await.unwrap();

    let snapshot = load_snapshot(&mut conn).await.unwrap();

    assert_eq!(
        snapshot,
        RegisterSnapshot {
            municipalities: vec![
                MunicipalitySnapshot {
                    id: OLD_ASKER,
                    number: "0220".into(),
                    name: "Asker".into(),
                    official_name: None,
                    county_number: "02".into(),
                    county_name: "Akershus".into(),
                    slug: "0220-asker".into(),
                    status: MunicipalityStatus::Dissolved,
                    source: MunicipalitySource::Kartverket,
                    names: vec![],
                },
                MunicipalitySnapshot {
                    id: BAERUM,
                    number: "3201".into(),
                    name: "Bærum".into(),
                    official_name: Some("Bærum".into()),
                    county_number: "32".into(),
                    county_name: "Akershus".into(),
                    slug: "3201-baerum".into(),
                    status: MunicipalityStatus::Active,
                    source: MunicipalitySource::Kartverket,
                    names: vec![OfficialName {
                        name: "Bærum".into(),
                        language: "no".into(),
                        priority: 1,
                    }],
                },
            ],
            schools: vec![
                SchoolSnapshot {
                    id: HOSLE,
                    municipality_id: BAERUM,
                    origin: Origin::Register,
                    orgnr: Some("974552124".into()),
                    register_name: Some("Hosle skole".into()),
                    display_name: "Hosle".into(),
                    display_name_curated: true,
                    slug: Some("hosle".into()),
                    verification: Verification::Listed,
                    status: SchoolStatus::Active,
                    in_scope: true,
                    scope_override: Some(false),
                    has_live_fau: true,
                    attributes: SchoolAttributes {
                        ownership: Some(Ownership::Public),
                        grade_from: Some(1),
                        grade_to: Some(7),
                        language: Some("nb".into()),
                        website: Some(String::new()),
                        street_address: Some("Bispeveien 73".into()),
                        postcode: Some("1362".into()),
                        post_town: None,
                    },
                },
                SchoolSnapshot {
                    id: SUBMITTED,
                    municipality_id: BAERUM,
                    origin: Origin::Submitted,
                    orgnr: None,
                    register_name: None,
                    display_name: "Nyskolen".into(),
                    display_name_curated: false,
                    slug: None,
                    verification: Verification::Pending,
                    status: SchoolStatus::Active,
                    in_scope: true,
                    scope_override: None,
                    has_live_fau: false,
                    attributes: SchoolAttributes::default(),
                },
            ],
            school_orgnr_history: vec![("975270920".into(), HOSLE)],
            school_slug_history: vec![(BAERUM, "hosle-skole".into(), HOSLE)],
            municipality_slug_history: vec![("3024-baerum".into(), BAERUM)],
        }
    );
}

#[tokio::test]
async fn an_empty_register_is_empty_until_it_holds_a_municipality() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    assert!(register_is_empty(&mut conn).await.unwrap());
    assert_eq!(
        load_snapshot(&mut conn).await.unwrap(),
        RegisterSnapshot::default()
    );

    common::register::test_municipality(&db.admin_pool()).await;
    assert!(!register_is_empty(&mut conn).await.unwrap());
}

/// `s` with every id relabelled: `municipality` for a municipality's id, `school` for a
/// school's.
fn relabel<A: RowId, B>(
    s: &RegisterSnapshot<A>,
    municipality: impl Fn(&A) -> B,
    school: impl Fn(&A) -> B,
) -> RegisterSnapshot<B> {
    RegisterSnapshot {
        municipalities: s
            .municipalities
            .iter()
            .map(|m| MunicipalitySnapshot {
                id: municipality(&m.id),
                number: m.number.clone(),
                name: m.name.clone(),
                official_name: m.official_name.clone(),
                county_number: m.county_number.clone(),
                county_name: m.county_name.clone(),
                slug: m.slug.clone(),
                status: m.status,
                source: m.source,
                names: m.names.clone(),
            })
            .collect(),
        schools: s
            .schools
            .iter()
            .map(|x| SchoolSnapshot {
                id: school(&x.id),
                municipality_id: municipality(&x.municipality_id),
                origin: x.origin,
                orgnr: x.orgnr.clone(),
                register_name: x.register_name.clone(),
                display_name: x.display_name.clone(),
                display_name_curated: x.display_name_curated,
                slug: x.slug.clone(),
                verification: x.verification,
                status: x.status,
                in_scope: x.in_scope,
                scope_override: x.scope_override,
                has_live_fau: x.has_live_fau,
                attributes: x.attributes.clone(),
            })
            .collect(),
        school_orgnr_history: s
            .school_orgnr_history
            .iter()
            .map(|(o, id)| (o.clone(), school(id)))
            .collect(),
        school_slug_history: s
            .school_slug_history
            .iter()
            .map(|(m, slug, id)| (municipality(m), slug.clone(), school(id)))
            .collect(),
        municipality_slug_history: s
            .municipality_slug_history
            .iter()
            .map(|(slug, m)| (slug.clone(), municipality(m)))
            .collect(),
    }
}

/// The database's register in the planner's test ids: every id, sorted, numbered from 1. The
/// order is kept, so the planner's id tie-breaks agree, and the test applier's `max + 1` ids
/// sort after every existing one, as UUIDv7s do.
fn to_u32(s: &RegisterSnapshot<Uuid>) -> RegisterSnapshot<u32> {
    let mut ids: Vec<Uuid> = s
        .municipalities
        .iter()
        .map(|m| m.id)
        .chain(s.schools.iter().map(|x| x.id))
        .collect();
    ids.sort();
    let map: HashMap<Uuid, u32> = ids.into_iter().zip(1..).collect();
    relabel(s, |id| map[id], |id| map[id])
}

/// A key per school that survives a change of id type: its orgnr, or else its display name.
fn school_keys<Id: RowId>(s: &RegisterSnapshot<Id>) -> HashMap<Id, String> {
    s.schools
        .iter()
        .map(|x| {
            let key = x.orgnr.clone().unwrap_or_else(|| x.display_name.clone());
            (x.id, format!("s{key}"))
        })
        .collect()
}

/// Ids replaced by keys that survive a change of id type (a municipality by its number, a
/// school by [`school_keys`]) and every list sorted, since the two appliers append in
/// different orders.
fn canonical<Id: RowId>(s: &RegisterSnapshot<Id>) -> RegisterSnapshot<String> {
    let m: HashMap<Id, String> = s
        .municipalities
        .iter()
        .map(|m| (m.id, format!("m{}", m.number)))
        .collect();
    let k = school_keys(s);
    let mut out = relabel(s, |id| m[id].clone(), |id| k[id].clone());
    for m in &mut out.municipalities {
        m.names.sort_by(|a, b| {
            (a.priority, &a.language, &a.name).cmp(&(b.priority, &b.language, &b.name))
        });
    }
    out.municipalities.sort_by(|a, b| a.id.cmp(&b.id));
    out.schools.sort_by(|a, b| a.id.cmp(&b.id));
    out.school_orgnr_history.sort();
    out.school_slug_history.sort();
    out.municipality_slug_history.sort();
    out
}

/// Every `schools.successor_id` link, as (closed school, successor) keys.
async fn successor_links(
    conn: &mut sqlx::PgConnection,
    snapshot: &RegisterSnapshot<Uuid>,
) -> BTreeSet<(String, String)> {
    let links: Vec<(Uuid, Uuid)> =
        sqlx::query_as("select id, successor_id from schools where successor_id is not null")
            .fetch_all(&mut *conn)
            .await
            .unwrap();
    let keys = school_keys(snapshot);
    links
        .into_iter()
        .map(|(closed, successor)| (keys[&closed].clone(), keys[&successor].clone()))
        .collect()
}

fn at(rfc3339: &str) -> Moment {
    Moment::at(rfc3339.parse().expect("an RFC 3339 timestamp"))
}

/// One run, both ways: plans the same inputs against the database's register and against its
/// u32 translation, applies the first with the SQL applier (as `fau_register`, in one
/// transaction, at `at`) and the second with the test applier, and asserts that the two
/// registers, and the successor links this run made, then agree, ids aside. Returns the SQL
/// side's outcome, and what its applier reported.
async fn step(
    pool: &PgPool,
    records: &[MunicipalityRecord],
    changes: &[CodeChange],
    units: &[NsrUnit],
    kind: RunKind,
    at: Moment,
) -> (SyncOutcome<Uuid>, Option<AppliedCounts>) {
    let mut tx = pool.begin().await.unwrap();
    let before = load_snapshot(&mut tx).await.unwrap();
    let links_before = successor_links(&mut tx, &before).await;
    let before_u32 = to_u32(&before);
    let inputs = testkit::inputs(records, changes, units, kind);
    let outcome = plan(&before, &inputs);
    let (expected, expected_links, applied) = match (&outcome, plan(&before_u32, &inputs)) {
        (SyncOutcome::Apply(sql_plan), SyncOutcome::Apply(test_plan)) => {
            let applied = apply_plan(&mut tx, sql_plan, at).await.unwrap();
            let (expected, successors) = testkit::apply(&before_u32, &test_plan);
            let keys = school_keys(&expected);
            let links: BTreeSet<(String, String)> = successors
                .iter()
                .map(|(closed, successor)| (keys[closed].clone(), keys[successor].clone()))
                .collect();
            (expected, links, Some(applied))
        }
        (SyncOutcome::NoChange, SyncOutcome::NoChange) => (before_u32, BTreeSet::new(), None),
        (a, b) => panic!("the two plans disagree: {a:?} against {b:?}"),
    };
    tx.commit().await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    let after = load_snapshot(&mut conn).await.unwrap();
    assert_eq!(canonical(&after), canonical(&expected));
    let links_after = successor_links(&mut conn, &after).await;
    assert_eq!(
        links_after
            .difference(&links_before)
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_links
    );
    (outcome, applied)
}

fn applied(outcome: SyncOutcome<Uuid>) -> SyncPlan<Uuid> {
    match outcome {
        SyncOutcome::Apply(plan) => plan,
        other => panic!("expected a plan, got {other:?}"),
    }
}

async fn text(admin: &PgPool, sql: &str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(admin).await.unwrap()
}

/// SQL for a timestamptz column as `2026-09-28T02:30:00Z`, whatever the session's time zone.
fn utc(column: &str) -> String {
    format!("to_char({column} at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')")
}

/// Kartverket's fixture municipalities with one change each: Bærum renumbered 3201 to 3299,
/// Heim renamed Heimdal (its official name and names follow), and Stange's official name
/// changed with its Norwegian name kept.
fn changed_records() -> Vec<MunicipalityRecord> {
    fixture_records()
        .into_iter()
        .map(|r| match r.number.as_str() {
            "3201" => record("3299", "Bærum", "32", "Akershus"),
            "5055" => record("5055", "Heimdal", "50", "Trøndelag"),
            "3413" => MunicipalityRecord {
                official_name: "Stange kommune".into(),
                names: vec![OfficialName {
                    name: "Stange kommune".into(),
                    language: "no".into(),
                    priority: 1,
                }],
                ..r
            },
            _ => r,
        })
        .collect()
}

#[tokio::test]
async fn municipality_ops_apply_like_the_test_applier() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    // Gran Canaria (2599) is out of scope: NSR is not empty, and no school is created.
    let units = [gran_canaria()];

    let seed = applied(
        step(
            &pool,
            &fixture_records(),
            &[],
            &units,
            RunKind::Seed,
            at("2026-09-28T02:30:00Z"),
        )
        .await
        .0,
    );
    assert_eq!(seed.counts.municipalities_created, 9);
    assert_eq!(
        text(
            &admin,
            "select search_text from municipalities where slug = '3201-baerum'"
        )
        .await,
        ["bærum baerum barum"]
    );

    let changes = [change("3201", "Bærum", "3299", "Bærum", date(2027, 1, 1))];
    let sync = applied(
        step(
            &pool,
            &changed_records(),
            &changes,
            &units,
            RunKind::Sync,
            at("2027-01-04T02:30:00Z"),
        )
        .await
        .0,
    );
    assert_eq!(
        (
            sync.counts.renumbered,
            sync.counts.renamed,
            sync.counts.municipalities_updated
        ),
        (1, 1, 2)
    );
    assert_eq!(
        text(
            &admin,
            &format!(
                "select slug || ' ' || {} || ' ' || {}
                   from municipality_slug_history order by slug",
                utc("valid_from"),
                utc("valid_until")
            )
        )
        .await,
        [
            "3201-baerum 2026-09-28T02:30:00Z 2027-01-04T02:30:00Z",
            "5055-heim 2026-09-28T02:30:00Z 2027-01-04T02:30:00Z",
        ]
    );
    assert_eq!(
        text(
            &admin,
            "select n.number || ' ' || n.valid_from || ' ' || coalesce(n.valid_until::text, '-')
               from municipality_numbers n join municipalities m on m.id = n.municipality_id
              where m.name = 'Bærum' order by n.valid_from"
        )
        .await,
        ["3201 2026-09-28 2027-01-01", "3299 2027-01-01 -"]
    );

    // The same sources again: nothing to do, on either side.
    assert_eq!(
        step(
            &pool,
            &changed_records(),
            &changes,
            &units,
            RunKind::Sync,
            at("2027-01-11T02:30:00Z"),
        )
        .await,
        (SyncOutcome::NoChange, None)
    );
}

/// The Handover's guard, which the test applier does not model: two renumbers in one run
/// write no history row for the middle slug, and a number first seen on the day of its
/// renumber closes a day after it opened.
#[tokio::test]
async fn a_middle_slug_writes_no_history_and_a_same_day_renumber_closes_a_day_later() {
    let db = TestDb::migrated().await;
    let pool = db.register_pool().await;
    let admin = db.admin_pool();
    let units = [gran_canaria()];
    let baerum = [record("3201", "Bærum", "32", "Akershus")];
    step(
        &pool,
        &baerum,
        &[],
        &units,
        RunKind::Seed,
        at("2027-01-01T08:00:00Z"),
    )
    .await;

    let changes = [
        change("3201", "Bærum", "3290", "Bærum", date(2027, 1, 1)),
        change("3290", "Bærum", "3291", "Bærum", date(2027, 6, 1)),
    ];
    let mut tx = pool.begin().await.unwrap();
    let before = load_snapshot(&mut tx).await.unwrap();
    let plan = applied(plan(
        &before,
        &testkit::inputs(
            &[record("3291", "Bærum", "32", "Akershus")],
            &changes,
            &units,
            RunKind::Sync,
        ),
    ));
    assert_eq!(plan.counts.renumbered, 2);
    apply_plan(&mut tx, &plan, at("2027-06-07T02:30:00Z"))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        text(
            &admin,
            &format!(
                "select slug || ' ' || {} || ' ' || {} from municipality_slug_history",
                utc("valid_from"),
                utc("valid_until")
            )
        )
        .await,
        ["3201-baerum 2027-01-01T08:00:00Z 2027-06-07T02:30:00Z"]
    );
    assert_eq!(
        text(
            &admin,
            "select number || ' ' || valid_from || ' ' || coalesce(valid_until::text, '-')
               from municipality_numbers order by valid_from"
        )
        .await,
        [
            "3201 2027-01-01 2027-01-02",
            "3290 2027-01-02 2027-06-01",
            "3291 2027-06-01 -",
        ]
    );
    assert_eq!(
        text(&admin, "select slug from municipalities").await,
        ["3291-baerum"]
    );
}
