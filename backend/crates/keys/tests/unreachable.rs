//! Spec §4.2 / §6: while OpenBao is sealed or unreachable, the backend still starts, and only
//! requests that need content fail (as `content_unavailable`). So `Keys::connect` must not
//! talk to OpenBao; the first operation is where a sealed or unreachable server surfaces.

use std::path::PathBuf;
use std::time::Duration;

use fau_crypto::Unit;
use fau_keys::{Auth, KeyError, Keys, KeysConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

/// A throwaway service-account JWT file, removed on drop. Kubernetes auth is the mode that
/// logs in; token auth never calls OpenBao before the first operation anyway.
struct Jwt(PathBuf);

impl Jwt {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("fau-keys-test-jwt-{}", Uuid::now_v7()));
        std::fs::write(&p, "not-a-real-jwt").unwrap();
        Self(p)
    }
}

impl Drop for Jwt {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn connect(address: String, jwt: &Jwt) -> Result<Keys, KeyError> {
    Keys::connect(KeysConfig {
        address,
        ca_cert_path: None,
        auth: Auth::Kubernetes {
            role: "fau-app".into(),
            jwt_path: jwt.0.clone(),
        },
        timeout: Duration::from_secs(2),
    })
    .await
}

fn record() -> Unit {
    Unit::Record {
        tenant: Uuid::now_v7(),
    }
}

#[tokio::test]
async fn connect_succeeds_while_openbao_is_unreachable_and_the_first_call_reports_it() {
    let jwt = Jwt::new();
    // Port 1 on loopback: nothing listens, so the connection is refused at once.
    let keys = connect("http://127.0.0.1:1".into(), &jwt)
        .await
        .expect("connect must not reach OpenBao");
    assert_eq!(keys.ensure_key(&record()).await, Err(KeyError::Unavailable));
}

/// A sealed OpenBao answers every request 503 `Vault is sealed`; the first call must say so.
#[tokio::test]
async fn a_sealed_openbao_surfaces_per_request_as_sealed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"errors":["Vault is sealed"]}"#;
                let resp = format!(
                    "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    let jwt = Jwt::new();
    let keys = connect(format!("http://127.0.0.1:{port}"), &jwt)
        .await
        .expect("connect must not reach OpenBao");
    assert_eq!(keys.ensure_key(&record()).await, Err(KeyError::Sealed));
    // Still sealed on the next request: nothing is cached from a failed login.
    assert_eq!(keys.ensure_key(&record()).await, Err(KeyError::Sealed));
}
