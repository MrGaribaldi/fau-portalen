//! A thin wrapper over vaultrs for exactly the transit calls the backend makes
//! (docs/key-service-design.md §3). The app policy allows nothing else anyway.

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use fau_crypto::{Aad, DataKey, MessageCiphertext, Unit, WrappedKey, Zeroizing};
use tokio::sync::RwLock;
use uuid::Uuid;
use vaultrs::api::transit::requests::{
    CreateKeyRequest, DataKeyType, DecryptDataRequest, EncryptDataRequest,
};
use vaultrs::client::{Client, VaultClient, VaultClientSettingsBuilder};
use vaultrs::error::ClientError;

const MOUNT: &str = "transit";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("OpenBao is sealed")]
    Sealed,
    #[error("no such key, or it has been shredded")]
    NotFound,
    #[error("OpenBao refused the request")]
    Forbidden,
    #[error("OpenBao's rate limit is reached")]
    RateLimited,
    #[error("the value could not be decrypted or the request was malformed")]
    Invalid,
    #[error("OpenBao is unreachable")]
    Unavailable,
}

pub(crate) fn map_err(e: ClientError) -> KeyError {
    match e {
        ClientError::APIError { code: 503, .. } => KeyError::Sealed,
        ClientError::APIError { code: 429, .. } => KeyError::RateLimited,
        ClientError::APIError { code: 403, .. } => KeyError::Forbidden,
        ClientError::APIError { code: 400, errors }
            if errors
                .iter()
                .any(|m| m.contains("encryption key not found") || m.contains("soft-deleted")) =>
        {
            KeyError::NotFound
        }
        ClientError::APIError { code: 400, .. } => KeyError::Invalid,
        _ => KeyError::Unavailable,
    }
}

pub enum Auth {
    /// Compose and tests only.
    Token(String),
    /// The cluster: the pod's service-account JWT, exchanged for a token under `role`.
    Kubernetes { role: String, jwt_path: PathBuf },
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::Token(_) => f.write_str("Token([redacted])"),
            Auth::Kubernetes { role, jwt_path } => f
                .debug_struct("Kubernetes")
                .field("role", role)
                .field("jwt_path", jwt_path)
                .finish(),
        }
    }
}

#[derive(Debug)]
pub struct KeysConfig {
    pub address: String,
    pub ca_cert_path: Option<String>,
    pub auth: Auth,
    pub timeout: Duration,
}

pub struct Keys {
    client: RwLock<VaultClient>,
    cfg: KeysConfig,
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keys").field("cfg", &self.cfg).finish()
    }
}

type Call<'a, T> = Pin<Box<dyn Future<Output = Result<T, ClientError>> + Send + 'a>>;

fn decode32(b64: &str) -> Result<DataKey, KeyError> {
    let bytes = Zeroizing::new(STANDARD.decode(b64).map_err(|_| KeyError::Invalid)?);
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| KeyError::Invalid)?;
    Ok(DataKey::from_bytes(arr))
}

impl Keys {
    pub async fn connect(cfg: KeysConfig) -> Result<Self, KeyError> {
        let mut s = VaultClientSettingsBuilder::default();
        s.address(&cfg.address).timeout(Some(cfg.timeout));
        if let Some(ca) = &cfg.ca_cert_path {
            s.ca_certs(vec![ca.clone()]);
        }
        if let Auth::Token(t) = &cfg.auth {
            s.token(t);
        }
        let client = VaultClient::new(s.build().map_err(|_| KeyError::Invalid)?)
            .map_err(|_| KeyError::Invalid)?;
        let keys = Self {
            client: RwLock::new(client),
            cfg,
        };
        keys.login().await?;
        Ok(keys)
    }

    async fn login(&self) -> Result<(), KeyError> {
        if let Auth::Kubernetes { role, jwt_path } = &self.cfg.auth {
            let jwt = Zeroizing::new(
                tokio::fs::read_to_string(jwt_path)
                    .await
                    .map_err(|_| KeyError::Unavailable)?,
            );
            let mut c = self.client.write().await;
            let info = vaultrs::auth::kubernetes::login(&*c, "kubernetes", role, jwt.trim())
                .await
                .map_err(map_err)?;
            c.set_token(&info.client_token);
        }
        Ok(())
    }

    /// One retry after re-login on 403, for an expired Kubernetes auth token (plan ruling 4).
    async fn call<T>(
        &self,
        f: impl for<'a> Fn(&'a VaultClient) -> Call<'a, T>,
    ) -> Result<T, KeyError> {
        let first = {
            let c = self.client.read().await;
            f(&c).await
        };
        match first {
            Err(ClientError::APIError { code: 403, .. })
                if matches!(self.cfg.auth, Auth::Kubernetes { .. }) =>
            {
                self.login().await?;
                let c = self.client.read().await;
                f(&c).await.map_err(map_err)
            }
            other => other.map_err(map_err),
        }
    }

    pub async fn ensure_key(&self, unit: &Unit) -> Result<(), KeyError> {
        let name = unit.key_name();
        self.call(|c| {
            let name = name.clone();
            Box::pin(async move {
                let mut b = CreateKeyRequest::builder();
                b.exportable(false).allow_plaintext_backup(false);
                vaultrs::transit::key::create(c, MOUNT, &name, Some(&mut b)).await
            })
        })
        .await
    }

    pub async fn new_data_key(&self, unit: &Unit) -> Result<(DataKey, WrappedKey), KeyError> {
        self.ensure_key(unit).await?;
        let name = unit.key_name();
        let r = self
            .call(|c| {
                let name = name.clone();
                Box::pin(async move {
                    vaultrs::transit::generate::data_key(
                        c,
                        MOUNT,
                        &name,
                        DataKeyType::Plaintext,
                        None,
                    )
                    .await
                })
            })
            .await?;
        let plaintext = Zeroizing::new(r.plaintext.ok_or(KeyError::Invalid)?);
        Ok((
            decode32(&plaintext)?,
            WrappedKey::new(r.ciphertext).ok_or(KeyError::Invalid)?,
        ))
    }

    pub async fn unwrap(&self, unit: &Unit, wrapped: &WrappedKey) -> Result<DataKey, KeyError> {
        let (name, ct) = (unit.key_name(), wrapped.as_str().to_owned());
        let r = self
            .call(|c| {
                let (name, ct) = (name.clone(), ct.clone());
                Box::pin(async move {
                    vaultrs::transit::data::decrypt(c, MOUNT, &name, &ct, None).await
                })
            })
            .await?;
        decode32(&Zeroizing::new(r.plaintext))
    }

    pub async fn encrypt_message(
        &self,
        tenant: Uuid,
        aad: &Aad,
        text: &str,
    ) -> Result<MessageCiphertext, KeyError> {
        let unit = Unit::Messages { tenant };
        self.ensure_key(&unit).await?;
        let (name, ad, pt) = (
            unit.key_name(),
            STANDARD.encode(aad.to_bytes()),
            Zeroizing::new(STANDARD.encode(text)),
        );
        let r = self
            .call(|c| {
                let (name, ad, pt) = (name.clone(), ad.clone(), pt.clone());
                Box::pin(async move {
                    let mut b = EncryptDataRequest::builder();
                    b.associated_data(ad);
                    vaultrs::transit::data::encrypt(c, MOUNT, &name, &pt, Some(&mut b)).await
                })
            })
            .await?;
        MessageCiphertext::new(r.ciphertext).ok_or(KeyError::Invalid)
    }

    pub async fn decrypt_message(
        &self,
        tenant: Uuid,
        aad: &Aad,
        ct: &MessageCiphertext,
    ) -> Result<Zeroizing<String>, KeyError> {
        let (name, ad, ct) = (
            Unit::Messages { tenant }.key_name(),
            STANDARD.encode(aad.to_bytes()),
            ct.as_str().to_owned(),
        );
        let r = self
            .call(|c| {
                let (name, ad, ct) = (name.clone(), ad.clone(), ct.clone());
                Box::pin(async move {
                    let mut b = DecryptDataRequest::builder();
                    b.associated_data(ad);
                    vaultrs::transit::data::decrypt(c, MOUNT, &name, &ct, Some(&mut b)).await
                })
            })
            .await?;
        let bytes = Zeroizing::new(
            STANDARD
                .decode(Zeroizing::new(r.plaintext).as_bytes())
                .map_err(|_| KeyError::Invalid)?,
        );
        Ok(Zeroizing::new(
            String::from_utf8(bytes.to_vec()).map_err(|_| KeyError::Invalid)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(code: u16, msg: &str) -> ClientError {
        ClientError::APIError {
            code,
            errors: vec![msg.to_owned()],
        }
    }

    #[test]
    fn openbao_errors_map_to_our_variants() {
        assert_eq!(map_err(api(503, "Vault is sealed")), KeyError::Sealed);
        assert_eq!(
            map_err(api(
                429,
                "request path \"transit/decrypt\": rate limit quota exceeded"
            )),
            KeyError::RateLimited
        );
        assert_eq!(map_err(api(403, "permission denied")), KeyError::Forbidden);
        assert_eq!(
            map_err(api(400, "encryption key not found")),
            KeyError::NotFound
        );
        assert_eq!(
            map_err(api(400, "refusing to use soft-deleted key")),
            KeyError::NotFound
        );
        assert_eq!(
            map_err(api(400, "cipher: message authentication failed")),
            KeyError::Invalid
        );
    }

    #[test]
    fn a_token_is_never_printed() {
        assert!(!format!("{:?}", Auth::Token("s.SECRET".into())).contains("SECRET"));
    }
}
