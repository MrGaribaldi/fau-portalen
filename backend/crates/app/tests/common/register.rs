//! Register fixtures. Inserted through a superuser connection to the caller's own
//! database, because neither the runtime role nor the harness's usual pools may
//! write listed schools (D9, RLS in 0004).

use sqlx::PgPool;
use uuid::Uuid;

/// The fixed test municipality, 0301 Oslo, with a stable id.
pub const TEST_MUNICIPALITY: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0301);

/// A superuser pool on the same database `pool` is connected to.
pub async fn admin_on_same_database(pool: &PgPool) -> PgPool {
    let db: String = sqlx::query_scalar("select current_database()")
        .fetch_one(pool)
        .await
        .expect("read current_database()");
    PgPool::connect(&super::with_database(&super::admin_url(), &db))
        .await
        .expect("connect as superuser to the test database")
}

/// Ensures the test municipality exists; idempotent.
pub async fn test_municipality(admin: &PgPool) -> Uuid {
    sqlx::query(
        "insert into municipalities (id, name, county_number, county_name, slug, status, source, search_text)
         values ($1, 'Oslo', '03', 'Oslo', '0301-oslo', 'active', 'kartverket', 'oslo')
         on conflict (id) do nothing",
    )
    .bind(TEST_MUNICIPALITY)
    .execute(admin)
    .await
    .expect("insert the test municipality");
    sqlx::query(
        "insert into municipality_numbers (municipality_id, number, valid_from)
         values ($1, '0301', '1838-01-01') on conflict do nothing",
    )
    .bind(TEST_MUNICIPALITY)
    .execute(admin)
    .await
    .expect("insert the test municipality's number");
    TEST_MUNICIPALITY
}

/// Ensures a listed school with this id exists in the test municipality; idempotent.
/// No slug, so any number of test schools can coexist.
pub async fn listed_school(admin: &PgPool, id: Uuid, label: &str) -> Uuid {
    let m = test_municipality(admin).await;
    // A nine-digit orgnr derived from the id, unique per id in practice.
    let orgnr = format!("{:09}", id.as_u128() % 1_000_000_000);
    sqlx::query(
        "insert into schools (id, municipality_id, origin, display_name, register_name,
                              verification, orgnr, status, search_text)
         values ($1, $2, 'register', $3, $3, 'listed', $4, 'active', $3)
         on conflict (id) do nothing",
    )
    .bind(id)
    .bind(m)
    .bind(label)
    .bind(orgnr)
    .execute(admin)
    .await
    .expect("insert a listed test school");
    id
}
