//! The whole chain against real Postgres and a real OpenBao (docs/key-service-design.md §7).

mod common;
use std::sync::Arc;
use std::time::Duration;

use common::membership::*;
use common::TestDb;
use fau_crypto::{decrypt, encrypt, Aad, Unit};
use fau_keys::{data_key, Auth, DataKeyError, KeyCache, KeyError, Keys, KeysConfig};
use fau_persistence::membership::*;
use jiff::SignedDuration;
use uuid::Uuid;

async fn keys() -> Arc<Keys> {
    Arc::new(
        Keys::connect(KeysConfig {
            address: std::env::var("TEST_OPENBAO_ADDR").unwrap(),
            ca_cert_path: None,
            auth: Auth::Token("dev-only-app".into()),
            timeout: Duration::from_secs(5),
        })
        .await
        .unwrap(),
    )
}

/// Stands in for the deletion job (Task 8): soft-delete every key of the FAU.
async fn soft_delete_fau(tenant: Uuid) {
    let addr = std::env::var("TEST_OPENBAO_ADDR").unwrap();
    let root = std::env::var("TEST_OPENBAO_TOKEN").unwrap();
    for name in [
        Unit::Record { tenant }.key_name(),
        Unit::Messages { tenant }.key_name(),
    ] {
        let s = reqwest::Client::new()
            .delete(format!("{addr}/v1/transit/keys/{name}/soft-delete"))
            .header("X-Vault-Token", &root)
            .send()
            .await
            .unwrap()
            .status();
        assert!(s.is_success(), "{name}");
    }
}

#[tokio::test]
async fn records_messages_and_shredding_work_end_to_end() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let keys = keys().await;
    let cache = KeyCache::new(
        keys.clone(),
        Arc::new(jiff::Timestamp::now),
        SignedDuration::from_mins(30),
    );
    let record = Unit::Record {
        tenant: fau.tenant_id,
    };

    // Envelope: the first session creates and stores the wrapped key; a second session unwraps it.
    let s1 = Uuid::now_v7();
    let k1 = {
        let mut c = pool.acquire().await.unwrap();
        data_key(&mut c, &keys, &cache, s1, &record).await.unwrap()
    };
    let row = Uuid::now_v7();
    let aad = Aad::new(fau.tenant_id, "groups", "encrypted_name", row);
    let ct = encrypt(&k1, &aad, "Juleballkomiteen").unwrap();
    let s2 = Uuid::now_v7();
    let k2 = {
        let mut c = pool.acquire().await.unwrap();
        data_key(&mut c, &keys, &cache, s2, &record).await.unwrap()
    };
    assert_eq!(
        decrypt(&k2, &aad, &ct).unwrap().as_str(),
        "Juleballkomiteen"
    );

    // No usable key material in Postgres: the plaintext data key appears nowhere in wrapped_keys.
    let stored: Vec<u8> =
        sqlx::query_scalar("select wrapped_key from wrapped_keys where tenant_id = $1")
            .bind(fau.tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!stored.windows(32).any(|w| w == k1.expose()));
    assert!(stored.starts_with(b"vault:v1:"));

    // An access-request message, encrypted with no session, read on the approval screen.
    let request_id = Uuid::now_v7();
    let (t, c) = ACCESS_REQUEST_MESSAGE_AAD;
    let msg_aad = Aad::new(fau.tenant_id, t, c, request_id);
    let sealed = keys
        .encrypt_message(fau.tenant_id, &msg_aad, "Jeg vil bli med i FAU")
        .await
        .unwrap();
    create_access_request(
        &pool,
        CreateAccessRequest {
            tenant_id: fau.tenant_id,
            requester: verified("ny@example.test"),
            message: Some(AccessRequestMessage {
                request_id,
                ciphertext: sealed,
            }),
        },
        t0,
    )
    .await
    .unwrap();
    let stored = access_request_message(
        &pool,
        fau.tenant_id,
        request_id,
        fau.admin_membership_id,
        t0,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        keys.decrypt_message(fau.tenant_id, &msg_aad, &stored)
            .await
            .unwrap()
            .as_str(),
        "Jeg vil bli med i FAU"
    );

    // Shredding: with the FAU's keys soft-deleted, a new session can read nothing.
    soft_delete_fau(fau.tenant_id).await;
    cache.release_session(s1);
    cache.release_session(s2);
    let s3 = Uuid::now_v7();
    let mut c = pool.acquire().await.unwrap();
    assert!(matches!(
        data_key(&mut c, &keys, &cache, s3, &record).await,
        Err(DataKeyError::Keys(KeyError::NotFound))
    ));
    assert_eq!(
        keys.decrypt_message(fau.tenant_id, &msg_aad, &stored)
            .await
            .unwrap_err(),
        KeyError::NotFound
    );
}
