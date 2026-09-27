//! `version (1) ‖ nonce (24) ‖ XChaCha20-Poly1305 ciphertext+tag`, with associated data
//! binding each value to its FAU, table, column and row (docs/key-service-design.md §3.2).

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::CryptoError;
use crate::key::DataKey;

const VERSION: u8 = 1;
const HEADER: usize = 1 + 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aad {
    pub tenant_id: Uuid,
    pub table: &'static str,
    pub column: &'static str,
    pub row_id: Uuid,
}

impl Aad {
    pub fn new(tenant_id: Uuid, table: &'static str, column: &'static str, row_id: Uuid) -> Self {
        Self {
            tenant_id,
            table,
            column,
            row_id,
        }
    }

    /// Length-prefixed, so ("ab","c") and ("a","bc") cannot collide. Also sent to OpenBao
    /// as transit `associated_data` for messages.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(b"fau-aad-v1");
        out.extend_from_slice(self.tenant_id.as_bytes());
        for part in [self.table, self.column] {
            out.extend_from_slice(&(part.len() as u32).to_be_bytes());
            out.extend_from_slice(part.as_bytes());
        }
        out.extend_from_slice(self.row_id.as_bytes());
        out
    }
}

/// Opaque stored ciphertext. Persistence functions for encrypted columns accept only this.
#[derive(Clone, PartialEq, Eq)]
pub struct Ciphertext(Vec<u8>);

impl Ciphertext {
    pub fn from_stored(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Ciphertext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ciphertext({} bytes)", self.0.len())
    }
}

pub fn encrypt(key: &DataKey, aad: &Aad, plaintext: &str) -> Result<Ciphertext, CryptoError> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| CryptoError::Randomness)?;
    let body = XChaCha20Poly1305::new(Key::from_slice(key.expose()))
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad.to_bytes(),
            },
        )
        .map_err(|_| CryptoError::Decrypt)?;
    let mut out = Vec::with_capacity(HEADER + body.len());
    out.push(VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&body);
    Ok(Ciphertext(out))
}

pub fn decrypt(
    key: &DataKey,
    aad: &Aad,
    ct: &Ciphertext,
) -> Result<Zeroizing<String>, CryptoError> {
    let b = &ct.0;
    if b.len() < HEADER + 16 || b[0] != VERSION {
        return Err(CryptoError::Malformed);
    }
    let plain = Zeroizing::new(
        XChaCha20Poly1305::new(Key::from_slice(key.expose()))
            .decrypt(
                XNonce::from_slice(&b[1..HEADER]),
                Payload {
                    msg: &b[HEADER..],
                    aad: &aad.to_bytes(),
                },
            )
            .map_err(|_| CryptoError::Decrypt)?,
    );
    let text = std::str::from_utf8(&plain).map_err(|_| CryptoError::Malformed)?;
    Ok(Zeroizing::new(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fixture() -> (DataKey, Aad) {
        (
            DataKey::from_bytes([9u8; 32]),
            Aad::new(Uuid::now_v7(), "groups", "encrypted_name", Uuid::now_v7()),
        )
    }

    #[test]
    fn round_trips() {
        let (key, aad) = fixture();
        let ct = encrypt(&key, &aad, "Juleballkomiteen").unwrap();
        assert_eq!(
            decrypt(&key, &aad, &ct).unwrap().as_str(),
            "Juleballkomiteen"
        );
    }

    #[test]
    fn a_value_moved_to_another_row_column_table_or_fau_does_not_decrypt() {
        let (key, aad) = fixture();
        let ct = encrypt(&key, &aad, "x").unwrap();
        for wrong in [
            Aad {
                row_id: Uuid::now_v7(),
                ..aad.clone()
            },
            Aad {
                column: "encrypted_title",
                ..aad.clone()
            },
            Aad {
                table: "events",
                ..aad.clone()
            },
            Aad {
                tenant_id: Uuid::now_v7(),
                ..aad.clone()
            },
        ] {
            assert_eq!(
                decrypt(&key, &wrong, &ct).unwrap_err(),
                CryptoError::Decrypt
            );
        }
    }

    #[test]
    fn the_associated_data_is_length_prefixed() {
        let t = Uuid::nil();
        let r = Uuid::nil();
        assert_ne!(
            Aad::new(t, "ab", "c", r).to_bytes(),
            Aad::new(t, "a", "bc", r).to_bytes()
        );
    }

    #[test]
    fn tampering_truncation_and_unknown_versions_are_refused() {
        let (key, aad) = fixture();
        let ct = encrypt(&key, &aad, "hei").unwrap();
        let mut b = ct.as_bytes().to_vec();
        *b.last_mut().unwrap() ^= 1;
        assert_eq!(
            decrypt(&key, &aad, &Ciphertext::from_stored(b)).unwrap_err(),
            CryptoError::Decrypt
        );
        assert_eq!(
            decrypt(&key, &aad, &Ciphertext::from_stored(vec![1, 2, 3])).unwrap_err(),
            CryptoError::Malformed
        );
        let mut v = ct.as_bytes().to_vec();
        v[0] = 2;
        assert_eq!(
            decrypt(&key, &aad, &Ciphertext::from_stored(v)).unwrap_err(),
            CryptoError::Malformed
        );
    }

    #[test]
    fn equal_plaintexts_encrypt_differently() {
        let (key, aad) = fixture();
        assert_ne!(
            encrypt(&key, &aad, "a").unwrap().as_bytes(),
            encrypt(&key, &aad, "a").unwrap().as_bytes()
        );
    }

    #[test]
    fn debug_prints_no_bytes() {
        let (key, aad) = fixture();
        let ct = encrypt(&key, &aad, "hemmelig").unwrap();
        assert_eq!(format!("{ct:?}"), format!("Ciphertext({} bytes)", ct.len()));
        assert_eq!(format!("{key:?}"), "DataKey([redacted])");
    }
}
