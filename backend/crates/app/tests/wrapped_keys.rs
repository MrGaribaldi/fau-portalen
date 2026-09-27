//! `wrapped_keys` (docs/key-service-design.md §3.2): one wrapped data key per unit, the
//! first writer wins, and nothing but OpenBao's ciphertext is stored.

mod common;
use common::membership::*;
use common::TestDb;
use fau_crypto::{ChatMonth, Unit, WrappedKey};
use fau_persistence::keys::{load_wrapped_key, store_wrapped_key, WrappedKeyError};
use uuid::Uuid;

fn wk(s: &str) -> WrappedKey {
    WrappedKey::new(format!("vault:v1:{s}")).unwrap()
}

#[tokio::test]
async fn the_first_stored_key_wins_and_is_loaded_back() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let mut conn = pool.acquire().await.unwrap();
    for unit in [
        Unit::Record {
            tenant: fau.tenant_id,
        },
        Unit::Document {
            tenant: fau.tenant_id,
            document: Uuid::now_v7(),
        },
        Unit::Chat {
            tenant: fau.tenant_id,
            month: ChatMonth::parse("2026-09").unwrap(),
        },
    ] {
        assert_eq!(load_wrapped_key(&mut conn, &unit).await.unwrap(), None);
        assert_eq!(
            store_wrapped_key(&mut conn, &unit, &wk("first"))
                .await
                .unwrap(),
            wk("first")
        );
        assert_eq!(
            store_wrapped_key(&mut conn, &unit, &wk("second"))
                .await
                .unwrap(),
            wk("first"),
            "{unit:?}: the loser gets the winner's key"
        );
        assert_eq!(
            load_wrapped_key(&mut conn, &unit).await.unwrap(),
            Some(wk("first"))
        );
    }
}

#[tokio::test]
async fn messages_have_no_data_key() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    assert!(matches!(
        load_wrapped_key(
            &mut conn,
            &Unit::Messages {
                tenant: Uuid::now_v7()
            }
        )
        .await,
        Err(WrappedKeyError::NoDataKey)
    ));
}

#[tokio::test]
async fn only_transit_ciphertext_is_accepted_by_the_schema() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let fau = active_fau(&db.app_pool().await, "admin@example.test", at(T0)).await;
    let bad = sqlx::query(
        "insert into wrapped_keys (tenant_id, unit, scope, wrapped_key) values ($1, 'record', null, $2)",
    )
    .bind(fau.tenant_id)
    .bind(b"raw key bytes".to_vec())
    .execute(&pool)
    .await;
    assert!(
        bad.is_err(),
        "a value without the vault:v prefix must be refused"
    );
    let bad_scope = sqlx::query(
        "insert into wrapped_keys (tenant_id, unit, scope, wrapped_key) values ($1, 'record', 'x', $2)",
    )
    .bind(fau.tenant_id)
    .bind(b"vault:v1:abc".to_vec())
    .execute(&pool)
    .await;
    assert!(bad_scope.is_err(), "a record key has no scope");
}

/// Spec §3.2: the key is `(tenant_id, unit, scope)`. A primary key cannot include the
/// nullable `scope` (a record key has none), so it is a table constraint with
/// `nulls not distinct`, which also covers the null-scope case in the same constraint.
#[tokio::test]
async fn the_table_is_keyed_on_tenant_unit_and_scope() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let key: Option<(String, bool)> = sqlx::query_as(
        "select string_agg(a.attname, ',' order by k.ord), bool_and(i.indnullsnotdistinct)
           from pg_constraint con
           join pg_index i on i.indexrelid = con.conindid
           cross join lateral unnest(con.conkey) with ordinality as k(attnum, ord)
           join pg_attribute a on a.attrelid = con.conrelid and a.attnum = k.attnum
          where con.conrelid = 'wrapped_keys'::regclass and con.contype in ('p', 'u')
          group by con.oid",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(key, Some(("tenant_id,unit,scope".into(), true)));

    let fau = active_fau(&db.app_pool().await, "admin@example.test", at(T0)).await;
    let insert = || {
        sqlx::query(
            "insert into wrapped_keys (tenant_id, unit, scope, wrapped_key) values ($1, 'record', null, $2)",
        )
        .bind(fau.tenant_id)
        .bind(b"vault:v1:abc".to_vec())
        .execute(&pool)
    };
    insert().await.unwrap();
    assert!(insert().await.is_err(), "one record key per FAU");
}
