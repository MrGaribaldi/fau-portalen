use std::sync::{Arc, Mutex};
use std::time::Duration;

use fau_crypto::Unit;
use fau_keys::{Auth, CacheClock, KeyCache, KeyError, Keys, KeysConfig};
use jiff::{SignedDuration, Timestamp};
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

fn clock() -> (Arc<Mutex<Timestamp>>, CacheClock) {
    let t = Arc::new(Mutex::new(Timestamp::now()));
    let t2 = t.clone();
    (t, Arc::new(move || *t2.lock().unwrap()))
}

fn advance(t: &Arc<Mutex<Timestamp>>, mins: i64) {
    let mut g = t.lock().unwrap();
    *g += SignedDuration::from_mins(mins);
}

async fn soft_delete(unit: &Unit) {
    let addr = std::env::var("TEST_OPENBAO_ADDR").unwrap();
    let root = std::env::var("TEST_OPENBAO_TOKEN").unwrap();
    let s = reqwest::Client::new()
        .delete(format!(
            "{addr}/v1/transit/keys/{}/soft-delete",
            unit.key_name()
        ))
        .header("X-Vault-Token", root)
        .send()
        .await
        .unwrap()
        .status();
    assert!(s.is_success());
}

#[tokio::test]
async fn a_session_unwraps_once_and_then_serves_from_memory() {
    let keys = keys().await;
    let (_t, c) = clock();
    let cache = KeyCache::new(keys.clone(), c, SignedDuration::from_mins(30));
    let unit = Unit::Record {
        tenant: Uuid::now_v7(),
    };
    let (key, wrapped) = keys.new_data_key(&unit).await.unwrap();
    let s = Uuid::now_v7();
    assert_eq!(
        cache.get(s, &unit, &wrapped).await.unwrap().expose(),
        key.expose()
    );
    soft_delete(&unit).await;
    assert_eq!(
        cache.get(s, &unit, &wrapped).await.unwrap().expose(),
        key.expose(),
        "served from memory: no second unwrap"
    );
    assert_eq!(
        cache
            .get(Uuid::now_v7(), &unit, &wrapped)
            .await
            .unwrap_err(),
        KeyError::NotFound,
        "a new session must unwrap, and the key is gone"
    );
}

#[tokio::test]
async fn idle_sessions_are_swept_sweeping_is_not_activity_and_touching_is() {
    let keys = keys().await;
    let (t, c) = clock();
    let cache = KeyCache::new(keys.clone(), c, SignedDuration::from_mins(30));
    let unit = Unit::Record {
        tenant: Uuid::now_v7(),
    };
    let (key, _) = keys.new_data_key(&unit).await.unwrap();
    let (idle, busy) = (Uuid::now_v7(), Uuid::now_v7());
    cache.put(idle, unit, key.clone());
    cache.put(busy, unit, key);
    advance(&t, 29);
    assert_eq!(cache.sweep(), 0);
    cache.touch_session(busy);
    advance(&t, 2);
    assert_eq!(
        cache.sweep(),
        1,
        "idle for 31 minutes; the sweep at 29 did not refresh it"
    );
    assert_eq!(cache.len(), 1, "the touched session survives");
}

#[tokio::test]
async fn logout_and_leaving_a_document_release_only_what_they_name() {
    let keys = keys().await;
    let (_t, c) = clock();
    let cache = KeyCache::new(keys.clone(), c, SignedDuration::from_mins(30));
    let tenant = Uuid::now_v7();
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    let (d1, d2) = (Uuid::now_v7(), Uuid::now_v7());
    let k = fau_crypto::DataKey::from_bytes([1; 32]);
    cache.put(a, Unit::Record { tenant }, k.clone());
    cache.put(
        a,
        Unit::Document {
            tenant,
            document: d1,
        },
        k.clone(),
    );
    cache.put(
        a,
        Unit::Document {
            tenant,
            document: d2,
        },
        k.clone(),
    );
    cache.put(b, Unit::Record { tenant }, k.clone());
    cache.release_document(a, tenant, d1);
    assert_eq!(cache.len(), 3);
    cache.release_session(a);
    assert_eq!(cache.len(), 1);
}

#[tokio::test]
async fn the_cache_debug_prints_no_key() {
    let (_t, c) = clock();
    let cache = KeyCache::new(keys().await, c, SignedDuration::from_mins(30));
    cache.put(
        Uuid::now_v7(),
        Unit::Record {
            tenant: Uuid::now_v7(),
        },
        fau_crypto::DataKey::from_bytes([0xAB; 32]),
    );
    let dbg = format!("{cache:?}");
    assert!(
        !dbg.contains("171") && !dbg.to_lowercase().contains("ab, ab"),
        "{dbg}"
    );
}
