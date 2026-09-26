//! `fau register export` (#3441 part 4, docs/school-register-design.md §9): the real binary,
//! as `fau_register`, against a register whose every row has a fixed id, compared with the
//! exact CSV.

mod common;

use common::register::{run_fau_register, test_municipality};
use common::TestDb;

#[tokio::test]
async fn the_export_lists_every_pickable_school_as_csv() {
    let db = TestDb::migrated().await;
    let admin = db.admin_pool();
    // 0301 Oslo, id ...0301.
    test_municipality(&admin).await;
    sqlx::raw_sql(
        "insert into municipalities (id, name, county_number, county_name, slug, status, source,
                                     search_text)
         values ('01990000-0000-7000-8000-000000003201', 'Bærum', '32', 'Akershus',
                 '3201-baerum', 'active', 'kartverket', 'bærum');
         insert into municipality_numbers (municipality_id, number, valid_from, valid_until)
         values ('01990000-0000-7000-8000-000000003201', '3024', '2020-01-01', '2024-01-01'),
                ('01990000-0000-7000-8000-000000003201', '3201', '2024-01-01', null);
         insert into schools (id, municipality_id, origin, display_name, register_name, slug,
                              verification, orgnr, in_scope, scope_override, status, closed_on,
                              closure_reason, search_text)
         values
           -- Listed, with a slug and a linked FAU.
           ('01990000-0000-7000-8000-000000000001', '01990000-0000-7000-8000-000000003201',
            'register', 'Hosle skole', 'Hosle skole', 'hosle-skole', 'listed', '974552124',
            true, null, 'active', null, null, 'hosle skole'),
           -- Listed, no slug, and a name that needs quoting.
           ('01990000-0000-7000-8000-000000000002', '01990000-0000-7000-8000-000000000301',
            'register', 'Skole \"Nord\", avd. 2', 'Skole Nord', null, 'listed', '900000002',
            true, null, 'active', null, null, 'skole nord'),
           -- Verified submission without an orgnr.
           ('01990000-0000-7000-8000-000000000003', '01990000-0000-7000-8000-000000000301',
            'submitted', 'Nyskolen', null, 'nyskolen', 'verified', null,
            true, null, 'active', null, null, 'nyskolen'),
           -- Out of scope by the filter, kept in by an operator.
           ('01990000-0000-7000-8000-000000000004', '01990000-0000-7000-8000-000000000301',
            'register', 'Sykehusskolen', 'Sykehusskolen', 'sykehusskolen', 'listed',
            '900000004', false, true, 'active', null, null, 'sykehusskolen'),
           -- Never exported: closed, held, pending, and in scope but overridden out.
           ('01990000-0000-7000-8000-000000000005', '01990000-0000-7000-8000-000000000301',
            'register', 'Nedlagt skole', 'Nedlagt skole', null, 'listed', '900000005',
            true, null, 'closed', '2026-01-01', 'closed', 'nedlagt skole'),
           ('01990000-0000-7000-8000-000000000006', '01990000-0000-7000-8000-000000000301',
            'register', 'Holdt skole', 'Holdt skole', null, 'held', '900000006',
            true, null, 'active', null, null, 'holdt skole'),
           ('01990000-0000-7000-8000-000000000007', '01990000-0000-7000-8000-000000000301',
            'submitted', 'Ventende skole', null, null, 'pending', null,
            true, null, 'active', null, null, 'ventende skole'),
           ('01990000-0000-7000-8000-000000000008', '01990000-0000-7000-8000-000000000301',
            'register', 'Voksenskolen', 'Voksenskolen', 'voksenskolen', 'listed', '900000008',
            true, false, 'active', null, null, 'voksenskolen');
         insert into registered_faus (orgnr, registered_name, organisation_form,
                                      municipality_number, status, last_seen_in_source_at)
         values ('913591100', 'FAU HOSLE SKOLE', 'FLI', '3201', 'active',
                 '2026-09-28T02:30:00Z');
         insert into school_fau_links (school_id, fau_orgnr, method, state)
         values ('01990000-0000-7000-8000-000000000001', '913591100', 'address', 'linked');",
    )
    .execute(&admin)
    .await
    .expect("the export fixture");

    let url = db.register_url();
    let out = run_fau_register(&["export"], &[("REGISTER_DATABASE_URL", url.as_str())]).await;
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "school_id,orgnr,municipality_number,display_name,path,fau_orgnr\n\
         01990000-0000-7000-8000-000000000002,900000002,0301,\"Skole \"\"Nord\"\", avd. 2\",/s/01990000-0000-7000-8000-000000000002,\n\
         01990000-0000-7000-8000-000000000003,,0301,Nyskolen,/fau/0301-oslo/nyskolen,\n\
         01990000-0000-7000-8000-000000000004,900000004,0301,Sykehusskolen,/fau/0301-oslo/sykehusskolen,\n\
         01990000-0000-7000-8000-000000000001,974552124,3201,Hosle skole,/fau/3201-baerum/hosle-skole,913591100\n"
    );
}
