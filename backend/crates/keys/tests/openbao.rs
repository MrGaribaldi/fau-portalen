//! Against the dev OpenBao from Task 2 (TEST_OPENBAO_ADDR, TEST_OPENBAO_TOKEN = root).
//! Every test uses a fresh tenant id, so tests never collide and nothing needs cleaning.

use std::time::Duration;

use fau_crypto::{Aad, ChatMonth, Unit};
use fau_keys::{Auth, KeyError, Keys, KeysConfig};
use reqwest::StatusCode;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Task 2's `sys/quotas/rate-limit/fau-transit` caps `transit/` at 50 req/s cluster-wide
/// (docs/key-service-design.md §4.3, tuned under #3442) — a production number, not a test-
/// suite one. Running these tests' `#[tokio::test]`s concurrently (the default) easily bursts
/// past it, so a forbidden-path assertion can observe 429 instead of 403 and fail for a reason
/// that has nothing to do with policy. This lock forces the file's tests to run one at a time;
/// it does not touch what any assertion checks.
static SERIAL: Mutex<()> = Mutex::const_new(());

fn addr() -> String {
    std::env::var("TEST_OPENBAO_ADDR").expect("TEST_OPENBAO_ADDR")
}
fn root() -> String {
    std::env::var("TEST_OPENBAO_TOKEN").expect("TEST_OPENBAO_TOKEN")
}

async fn app() -> Keys {
    Keys::connect(KeysConfig {
        address: addr(),
        ca_cert_path: None,
        auth: Auth::Token("dev-only-app".into()),
        timeout: Duration::from_secs(5),
    })
    .await
    .unwrap()
}

/// Raw calls for what the app must not do, or what only root/operator does.
async fn raw(
    token: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> StatusCode {
    let mut r = reqwest::Client::new()
        .request(method, format!("{}/v1/{path}", addr()))
        .header("X-Vault-Token", token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap().status()
}

#[tokio::test]
async fn a_data_key_unwraps_to_the_same_bytes() {
    let _serial = SERIAL.lock().await;
    let keys = app().await;
    let unit = Unit::Record {
        tenant: Uuid::now_v7(),
    };
    let (key, wrapped) = keys.new_data_key(&unit).await.unwrap();
    assert!(wrapped.as_str().starts_with("vault:v1:"));
    assert_eq!(
        keys.unwrap(&unit, &wrapped).await.unwrap().expose(),
        key.expose()
    );
}

#[tokio::test]
async fn a_wrapped_key_opens_only_under_its_own_unit() {
    let _serial = SERIAL.lock().await;
    let keys = app().await;
    let tenant = Uuid::now_v7();
    let (_, wrapped) = keys.new_data_key(&Unit::Record { tenant }).await.unwrap();
    let other = Unit::Chat {
        tenant,
        month: ChatMonth::parse("2026-09").unwrap(),
    };
    keys.ensure_key(&other).await.unwrap();
    assert_eq!(
        keys.unwrap(&other, &wrapped).await.unwrap_err(),
        KeyError::Invalid
    );
    assert_eq!(
        keys.unwrap(
            &Unit::Record {
                tenant: Uuid::now_v7()
            },
            &wrapped
        )
        .await
        .unwrap_err(),
        KeyError::NotFound
    );
}

#[tokio::test]
async fn messages_round_trip_and_are_bound_to_their_row() {
    let _serial = SERIAL.lock().await;
    let keys = app().await;
    let tenant = Uuid::now_v7();
    let aad = Aad::new(
        tenant,
        "access_requests",
        "encrypted_message",
        Uuid::now_v7(),
    );
    let ct = keys
        .encrypt_message(tenant, &aad, "Jeg vil bli med i FAU")
        .await
        .unwrap();
    assert_eq!(
        keys.decrypt_message(tenant, &aad, &ct)
            .await
            .unwrap()
            .as_str(),
        "Jeg vil bli med i FAU"
    );
    let moved = Aad {
        row_id: Uuid::now_v7(),
        ..aad
    };
    assert_eq!(
        keys.decrypt_message(tenant, &moved, &ct).await.unwrap_err(),
        KeyError::Invalid
    );
}

#[tokio::test]
async fn soft_delete_stops_use_restore_brings_it_back_and_hard_delete_is_final() {
    let _serial = SERIAL.lock().await;
    let keys = app().await;
    let unit = Unit::Record {
        tenant: Uuid::now_v7(),
    };
    let (_, wrapped) = keys.new_data_key(&unit).await.unwrap();
    let name = unit.key_name();
    let root = root();
    assert!(raw(
        &root,
        reqwest::Method::DELETE,
        &format!("transit/keys/{name}/soft-delete"),
        None
    )
    .await
    .is_success());
    assert_eq!(
        keys.unwrap(&unit, &wrapped).await.unwrap_err(),
        KeyError::NotFound
    );
    assert_eq!(
        keys.new_data_key(&unit).await.unwrap_err(),
        KeyError::NotFound,
        "creating does not resurrect a soft-deleted key"
    );
    assert!(raw(
        &root,
        reqwest::Method::POST,
        &format!("transit/keys/{name}/soft-delete-restore"),
        None
    )
    .await
    .is_success());
    assert!(keys.unwrap(&unit, &wrapped).await.is_ok());
    assert!(raw(
        &root,
        reqwest::Method::POST,
        &format!("transit/keys/{name}/config"),
        Some(serde_json::json!({"deletion_allowed": true}))
    )
    .await
    .is_success());
    assert!(raw(
        &root,
        reqwest::Method::DELETE,
        &format!("transit/keys/{name}"),
        None
    )
    .await
    .is_success());
    assert_eq!(
        keys.unwrap(&unit, &wrapped).await.unwrap_err(),
        KeyError::NotFound
    );
    keys.ensure_key(&unit).await.unwrap();
    assert_eq!(
        keys.unwrap(&unit, &wrapped).await.unwrap_err(),
        KeyError::Invalid,
        "a re-created key of the same name does not open old wrapped keys"
    );
}

#[tokio::test]
async fn the_policies_allow_exactly_what_the_spec_says() {
    let _serial = SERIAL.lock().await;
    use reqwest::Method as M;
    let keys = app().await;
    let unit = Unit::Record {
        tenant: Uuid::now_v7(),
    };
    let (_, wrapped) = keys.new_data_key(&unit).await.unwrap();
    let k = unit.key_name();
    let decrypt = Some(serde_json::json!({ "ciphertext": wrapped.as_str() }));
    let denied = |s: StatusCode| s == StatusCode::FORBIDDEN;

    // The app: everything but create / datakey / encrypt / decrypt is refused.
    let app_refused: Vec<(M, String, Option<serde_json::Value>)> = vec![
        (M::GET, format!("transit/keys/{k}"), None),
        (M::from_bytes(b"LIST").unwrap(), "transit/keys".into(), None),
        (M::DELETE, format!("transit/keys/{k}"), None),
        (
            M::POST,
            format!("transit/keys/{k}/config"),
            Some(serde_json::json!({"deletion_allowed": true})),
        ),
        (M::POST, format!("transit/keys/{k}/rotate"), None),
        (
            M::POST,
            format!("transit/keys/{k}/trim"),
            Some(serde_json::json!({"min_available_version": 1})),
        ),
        (M::DELETE, format!("transit/keys/{k}/soft-delete"), None),
        (
            M::POST,
            format!("transit/keys/{k}/soft-delete-restore"),
            None,
        ),
        (M::GET, format!("transit/export/encryption-key/{k}"), None),
        (M::GET, format!("transit/backup/{k}"), None),
        (
            M::POST,
            format!("transit/encrypt/fau-{}-messages", Uuid::now_v7()),
            Some(serde_json::json!({"plaintext": "aGVp"})),
        ),
        (
            M::POST,
            "fau-keys-queue/x".into(),
            Some(serde_json::json!({"a": "b"})),
        ),
    ];
    for (m, p, b) in app_refused {
        assert!(
            denied(raw("dev-only-app", m.clone(), &p, b).await),
            "app must not {m} {p}"
        );
    }
    // The operator: can destroy, can never read data.
    for (m, p, b) in [
        (M::POST, format!("transit/decrypt/{k}"), decrypt.clone()),
        (M::POST, format!("transit/datakey/plaintext/{k}"), None),
        (
            M::POST,
            format!("transit/encrypt/{k}"),
            Some(serde_json::json!({"plaintext": "aGVp"})),
        ),
    ] {
        assert!(
            denied(raw("dev-only-operator", m.clone(), &p, b).await),
            "operator must not {m} {p}"
        );
    }
    assert!(
        raw(
            "dev-only-operator",
            M::GET,
            &format!("transit/keys/{k}"),
            None
        )
        .await
        .is_success(),
        "operator reads metadata"
    );
    assert!(raw(
        "dev-only-operator",
        M::from_bytes(b"LIST").unwrap(),
        "transit/keys",
        None
    )
    .await
    .is_success());
}

/// The previous test only ever calls the operator's *allowed* config/soft-delete/restore/hard-
/// delete/queue paths with the root token, which bypasses policy entirely — so an over- or
/// under-scoped `fau-keys-operator` policy would go unnoticed. Here every one of those calls uses
/// the `dev-only-operator` token itself, on a key the app client created.
#[tokio::test]
async fn the_operator_can_destroy_and_queue_with_its_own_token() {
    let _serial = SERIAL.lock().await;
    use reqwest::Method as M;
    let keys = app().await;
    let unit = Unit::Record {
        tenant: Uuid::now_v7(),
    };
    let (_, wrapped) = keys.new_data_key(&unit).await.unwrap();
    let k = unit.key_name();
    let op = "dev-only-operator";

    // 1. Allow hard delete up front.
    assert!(raw(
        op,
        M::POST,
        &format!("transit/keys/{k}/config"),
        Some(serde_json::json!({"deletion_allowed": true}))
    )
    .await
    .is_success());

    // 2. Soft-delete stops the app from using the key.
    assert!(raw(
        op,
        M::DELETE,
        &format!("transit/keys/{k}/soft-delete"),
        None
    )
    .await
    .is_success());
    assert_eq!(
        keys.unwrap(&unit, &wrapped).await.unwrap_err(),
        KeyError::NotFound
    );

    // 3. Restore brings it back.
    assert!(raw(
        op,
        M::POST,
        &format!("transit/keys/{k}/soft-delete-restore"),
        None
    )
    .await
    .is_success());
    assert!(keys.unwrap(&unit, &wrapped).await.is_ok());

    // 4. Soft-delete again, then hard delete (deletion_allowed already true) is final.
    assert!(raw(
        op,
        M::DELETE,
        &format!("transit/keys/{k}/soft-delete"),
        None
    )
    .await
    .is_success());
    assert!(raw(op, M::DELETE, &format!("transit/keys/{k}"), None)
        .await
        .is_success());
    assert_eq!(
        keys.unwrap(&unit, &wrapped).await.unwrap_err(),
        KeyError::NotFound
    );

    // 5. The 7-day deletion queue, exercised end to end with the operator's own token.
    assert!(raw(
        op,
        M::POST,
        &format!("fau-keys-queue/{k}"),
        Some(serde_json::json!({"soft_deleted_at": "1790500000"}))
    )
    .await
    .is_success());
    assert!(raw(op, M::GET, &format!("fau-keys-queue/{k}"), None)
        .await
        .is_success());
    assert!(
        raw(op, M::from_bytes(b"LIST").unwrap(), "fau-keys-queue", None)
            .await
            .is_success()
    );
    assert!(raw(op, M::DELETE, &format!("fau-keys-queue/{k}"), None)
        .await
        .is_success());

    // 6. The operator cannot write outside its granted paths.
    let unit2 = Unit::Record {
        tenant: Uuid::now_v7(),
    };
    let k2 = unit2.key_name();
    assert_eq!(
        raw(op, M::POST, &format!("transit/keys/{k2}"), None).await,
        StatusCode::FORBIDDEN,
        "operator must not create a transit key"
    );
    assert_eq!(
        raw(
            op,
            M::POST,
            "sys/policies/acl/x",
            Some(serde_json::json!({"policy": "path \"*\" { capabilities = [\"read\"] }"}))
        )
        .await,
        StatusCode::FORBIDDEN,
        "operator must not write policies"
    );
}
