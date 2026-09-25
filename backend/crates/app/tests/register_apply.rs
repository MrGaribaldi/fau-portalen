//! The register's persistence (#3441 part 4): the snapshot loader and the SQL applier,
//! against real PostgreSQL as `fau_register`, checked for parity with the planner's in-memory
//! test applier (`fau_domain::register::sync::testkit::apply`).

mod common;

use common::TestDb;
use fau_domain::register::source::OfficialName;
use fau_domain::register::sync::{
    MunicipalitySnapshot, MunicipalitySource, MunicipalityStatus, Origin, Ownership,
    RegisterSnapshot, SchoolAttributes, SchoolSnapshot, SchoolStatus, Verification,
};
use fau_persistence::register::{load_snapshot, register_is_empty};
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
