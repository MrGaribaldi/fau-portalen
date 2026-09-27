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
