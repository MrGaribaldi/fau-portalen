# Key Service Implementation Plan (#3506)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the ADR-003 key service as a separate, sealed-on-start binary with its own SQLite store, the backend crates that talk to it and encrypt with its keys, and the two #3418 messages as its first consumers.

**Architecture:** Four new crates:
- `fau-crypto`: backend-side primitives (envelope, field cipher, HPKE sealing).
- `fau-key-protocol`: serde wire types shared by client and server.
- `fau-key-service`: binary `fau-keys` (SQLite store, vault, leases, mTLS API, unseal listener, metrics).
- `fau-key-client`: mTLS client and the session-held `KeyCache`.

Content is encrypted in the backend; the key service only hands out one data key per call, under a lease, to a caller authenticated by client certificate. Persistence stores ciphertext as `bytea` and accepts only the opaque `Ciphertext` / `Sealed` types.

**Tech Stack:** Rust 1.98.1, tokio, axum 0.8, hyper 1 / hyper-util, rustls 0.23 (ring provider) + tokio-rustls 0.26, reqwest 0.13 (rustls), sqlx 0.8 (sqlite for the key service, postgres for the app), chacha20poly1305 0.10, hpke 0.12, zeroize 1, rcgen 0.14, x509-parser 0.18, jiff 0.2.

**Spec:** `docs/key-service-design.md` (read it first; section numbers below refer to it). Background: `docs/identity-and-encryption.md` decisions 5, 5a, 6, 7 and 7b.

## Global Constraints

- **The root key, a KEK, a data key, a nonce or a ciphertext never appears in a log line, a
  metric, an error message, a `Debug` output or a panic message.** Every type holding one has a
  hand-written redacting `Debug`.
- **Endpoints:** there are no list, bulk or export endpoints. No endpoint returns more than one key,
  and no endpoint returns a KEK or the root key (§3.2).
- **Store:** SQLite with `secure_delete = ON`, `journal_mode = DELETE` and `synchronous = FULL`, one
  connection. Destroying a key deletes its rows (§4.1).
- **Dev mode:** `FAU_KEYS_DEV_ROOT_KEY` is honoured only with the `dev` cargo feature. Without it,
  the variable's presence is a startup error (§6.4).
- **Unseal listener:** it must bind a loopback address; a non-loopback bind is a configuration
  error (§4.5).
- **Chat epochs:** only the current Europe/Oslo month may be created, and a future month is
  refused. Destroying one is refused unless it ended at least 12 months ago (§3.2).
- **Leases:** the idle timeout defaults to 1800 s, and the ceiling to 200 distinct FAUs per backend
  instance. `release` never counts as activity (§3.3).
- **Configuration errors** name the variable and never the value (the existing app convention in
  `crates/app/src/config.rs`).
- **Crypto versions are pinned** to the exact APIs verified on 26 September 2026:
  `chacha20poly1305 = "0.10"`, `hpke = "0.12"` (features `alloc`, `x25519`),
  `rand_core = "0.6"` (feature `getrandom`), `rustls = "0.23"`, `tokio-rustls = "0.26"`,
  `rcgen = "0.14"`, `x509-parser = "0.18"`. Do not upgrade them in this plan.
- **Language:** all code, comments and docs are in English. User-facing strings are Bokmål, and
  there is exactly one here: the sealed page text in Task 9.
- **Commits:** end every message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  If git reports no identity, pass it per command:
  `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as` and the same for
  `GIT_COMMITTER_*`. Never change git config. Never push.
- **Test commands** run from `/workspace/backend`. Suites that touch PostgreSQL need
  `TEST_DATABASE_URL` (see `docs/app-foundation-operations.md`). The key-service suites need no
  PostgreSQL.
- **Never run `fau-keys init` or `unseal` against anything but a throwaway test store.** Their
  output is a root key, and an agent transcript is persistent (§4.3). Tests use temp directories
  only.

## Rulings this plan makes (for Erik's review)

These choices were made while planning and never discussed. Each one is cheap to change before
execution.

1. **A sealed key service answers the error code `content_unavailable`, not a page.** The API
   error contract sends codes and never prose. The Bokmål sentence from spec §5.4 becomes the
   code's catalogue string (#3439). Task 10.
2. **The app does not depend on `keys` in compose yet.** Nothing in the app calls the key service
   until #3417 or #3501 adds the first HTTP consumer, which also wires `KEY_SERVICE_*`
   configuration. Task 11.
3. **In dev builds, `FAU_KEYS_DEV_ROOT_KEY` is a passphrase** that the root key is derived from,
   so compose can carry the obviously fake default `local-dev-only`. The release image refuses
   the variable either way. Task 7.
4. **`KeyCache` is built and tested here, but hooked into request middleware by #3417**, where
   sessions first exist. Task 8.
5. **`destroy_fau` removes only the KEK from the live store, and keeps the keys wrapped under
   it.** During the 7-day window, a KEK recovered from the replica (#3507) must still find them.
   Without the KEK they are unreadable. Task 3.
6. **Crypto crate versions are pinned** to the APIs that were compiled and run on 26 September
   (chacha20poly1305 0.10, hpke 0.12, rustls 0.23). Upgrading them is separate work.

## File Structure

```
backend/
  Cargo.toml                                  modify: members + workspace deps
  crates/
    crypto/                                   NEW fau-crypto
      Cargo.toml
      src/lib.rs                              re-exports
      src/key.rs                              DataKey, KeyId
      src/envelope.rs                         Aad, Ciphertext, encrypt, decrypt, key_id_of
      src/seal.rs                             SealingPublicKey/PrivateKey, Sealed, seal, open
      src/error.rs                            CryptoError
    key-protocol/                             NEW fau-key-protocol (serde only)
      Cargo.toml
      src/lib.rs                              KeyClass, requests, responses, ErrorBody, ROUTES
    key-service/                              NEW fau-key-service, bin fau-keys
      Cargo.toml
      migrations/0001_store.sql
      src/lib.rs                              module list, test support
      src/root.rs                             RootKey, encoding + checksum
      src/wrap.rs                             XChaCha wrap/unwrap with AAD
      src/store.rs                            SQLite Store
      src/vault.rs                            Vault: sealed/unsealed, canary
      src/leases.rs                           LeaseTable, ceiling
      src/limits.rs                           RateLimiter (token buckets)
      src/epoch.rs                            chat-epoch month rules (Europe/Oslo)
      src/metrics.rs                          counters + Prometheus text
      src/api.rs                              axum router, handlers, error mapping
      src/tls.rs                              mTLS accept loop, PeerIdentity
      src/unseal.rs                           loopback unseal router
      src/devcerts.rs                         rcgen dev CA + certs
      src/config.rs                           env config
      src/hardening.rs                        PR_SET_DUMPABLE, RLIMIT_CORE
      src/main.rs                             serve / init / unseal / dev-certs
      tests/api.rs                            in-process server tests
      tests/binary.rs                         release-guard + init/unseal via the binary
    key-client/                               NEW fau-key-client
      Cargo.toml
      src/lib.rs
      src/client.rs                           KeyClient, KeyError
      src/cache.rs                            KeyCache, leases, sweeper
      tests/cache.rs                          against the real in-process service
    persistence/src/membership/requests.rs    modify: sealed access-request message
    persistence/src/membership/invitations.rs modify: encrypted invitation message
    app/src/http/error.rs                     modify: the sealed ("utilgjengelig") page
    app/tests/requests.rs                     modify
    app/tests/invitations.rs                  modify
    app/tests/key_chain.rs                    NEW end-to-end: key service + Postgres
Dockerfile                                    modify: key-service target
compose.yaml                                  modify: keys service
gitops/apps/key-service/*.yaml                NEW manifests (deployed by #3424)
docs/key-service-operations.md               NEW init/unseal runbook, alert rules
.gitignore                                    modify: dev certs dir
```

---
### Task 1: `fau-crypto`, the backend's envelope and sealing

**Files:**
- Modify: `backend/Cargo.toml` (members, workspace deps)
- Create: `backend/crates/crypto/Cargo.toml`, `src/{lib,key,envelope,seal,error}.rs`
- Test: unit tests inside each module

**Interfaces:**
- Produces:
  - `DataKey`: `from_bytes([u8; 32])`, `generate() -> Result<DataKey, CryptoError>`, `expose() -> &[u8; 32]`.
  - `KeyId(Uuid)`: `KeyId::new(Uuid)`, `.uuid()`.
  - `Aad::new(tenant_id: Uuid, table: &'static str, column: &'static str, row_id: Uuid)`.
  - `Ciphertext`: `from_stored(Vec<u8>)`, `as_bytes()`, `len()`.
  - `encrypt(key_id: KeyId, key: &DataKey, aad: &Aad, plaintext: &str) -> Result<Ciphertext, CryptoError>`.
  - `decrypt(key: &DataKey, aad: &Aad, ct: &Ciphertext) -> Result<Zeroizing<String>, CryptoError>`.
  - `key_id_of(ct: &Ciphertext) -> Result<KeyId, CryptoError>`.
  - `SealingPublicKey::from_bytes([u8; 32])` / `to_bytes()`.
  - `SealingPrivateKey::from_bytes([u8; 32])`.
  - `Sealed`: `from_stored(Vec<u8>)`, `as_bytes()`.
  - `seal(pk: &SealingPublicKey, aad: &Aad, plaintext: &str) -> Result<Sealed, CryptoError>`.
  - `open(sk: &SealingPrivateKey, aad: &Aad, sealed: &Sealed) -> Result<Zeroizing<String>, CryptoError>`.
  - `generate_sealing_pair() -> (SealingPublicKey, SealingPrivateKey)`.
  - `CryptoError { Randomness, Malformed, Decrypt }`.

- [ ] **Step 1: Add the crate to the workspace**

In `backend/Cargo.toml`:
- set `members = ["crates/domain", "crates/persistence", "crates/app", "crates/register-sources", "crates/crypto", "crates/key-protocol", "crates/key-service", "crates/key-client"]`;
- add these to `[workspace.dependencies]`:

```toml
chacha20poly1305 = "0.10"
hpke = { version = "0.12", default-features = false, features = ["alloc", "x25519"] }
rand_core = { version = "0.6", features = ["getrandom"] }
zeroize = "1"
base64 = "0.22"
```

The other three member crates are created in Tasks 2, 3 and 8. Until then, create each as an
empty stub so the workspace builds. A stub is a `Cargo.toml` with `[package] name = "fau-key-protocol"` (resp. `fau-key-service`, `fau-key-client`), `version.workspace = true`, `edition.workspace = true`, `license.workspace = true`, `publish = false`, plus an empty `src/lib.rs`.

`backend/crates/crypto/Cargo.toml`:

```toml
[package]
name = "fau-crypto"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

# Backend-side encryption (docs/key-service-design.md §5.1). No I/O, no HTTP, no SQL:
# keys come in from fau-key-client, ciphertext goes out to fau-persistence.
[dependencies]
chacha20poly1305 = { workspace = true }
getrandom = { workspace = true }
hpke = { workspace = true }
rand_core = { workspace = true }
uuid = { workspace = true }
zeroize = { workspace = true }
```

- [ ] **Step 2: Write the failing tests** (`src/envelope.rs`, bottom of the file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{DataKey, KeyId};
    use uuid::Uuid;

    fn fixture() -> (KeyId, DataKey, Aad) {
        let t = Uuid::now_v7();
        (KeyId::new(Uuid::now_v7()), DataKey::from_bytes([9u8; 32]), Aad::new(t, "groups", "encrypted_name", Uuid::now_v7()))
    }

    #[test]
    fn round_trips_and_names_its_key() {
        let (id, key, aad) = fixture();
        let ct = encrypt(id, &key, &aad, "Juleballkomiteen").unwrap();
        assert_eq!(key_id_of(&ct).unwrap(), id);
        assert_eq!(decrypt(&key, &aad, &ct).unwrap().as_str(), "Juleballkomiteen");
    }

    #[test]
    fn a_value_moved_to_another_row_column_or_fau_does_not_decrypt() {
        let (id, key, aad) = fixture();
        let ct = encrypt(id, &key, &aad, "x").unwrap();
        let other_row = Aad { row_id: Uuid::now_v7(), ..aad.clone() };
        let other_col = Aad { column: "encrypted_title", ..aad.clone() };
        let other_fau = Aad { tenant_id: Uuid::now_v7(), ..aad.clone() };
        let other_table = Aad { table: "events", ..aad.clone() };
        for wrong in [other_row, other_col, other_fau, other_table] {
            assert_eq!(decrypt(&key, &wrong, &ct).unwrap_err(), CryptoError::Decrypt);
        }
    }

    #[test]
    fn tampering_and_truncation_are_refused() {
        let (id, key, aad) = fixture();
        let ct = encrypt(id, &key, &aad, "hei").unwrap();
        let mut bytes = ct.as_bytes().to_vec();
        *bytes.last_mut().unwrap() ^= 1;
        assert_eq!(decrypt(&key, &aad, &Ciphertext::from_stored(bytes)).unwrap_err(), CryptoError::Decrypt);
        assert_eq!(decrypt(&key, &aad, &Ciphertext::from_stored(vec![1, 2, 3])).unwrap_err(), CryptoError::Malformed);
        let mut wrong_version = ct.as_bytes().to_vec();
        wrong_version[0] = 2;
        assert_eq!(key_id_of(&Ciphertext::from_stored(wrong_version)).unwrap_err(), CryptoError::Malformed);
    }

    #[test]
    fn two_encryptions_of_the_same_text_differ() {
        let (id, key, aad) = fixture();
        assert_ne!(encrypt(id, &key, &aad, "a").unwrap().as_bytes(), encrypt(id, &key, &aad, "a").unwrap().as_bytes());
    }

    #[test]
    fn debug_never_prints_bytes() {
        let (id, key, aad) = fixture();
        let ct = encrypt(id, &key, &aad, "hemmelig").unwrap();
        assert_eq!(format!("{ct:?}"), format!("Ciphertext({} bytes)", ct.len()));
        assert_eq!(format!("{key:?}"), "DataKey([redacted])");
    }

    #[test]
    fn a_500_character_message_fits_the_2200_byte_column() {
        let (id, key, aad) = fixture();
        assert!(encrypt(id, &key, &aad, &"ø".repeat(500)).unwrap().len() <= 2200);
    }
}
```

In `src/seal.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::Aad;
    use uuid::Uuid;

    #[test]
    fn sealed_text_opens_only_with_the_private_key_and_the_same_aad() {
        let (pk, sk) = generate_sealing_pair();
        let aad = Aad::new(Uuid::now_v7(), "access_requests", "sealed_message", Uuid::now_v7());
        let sealed = seal(&pk, &aad, "Hei, jeg er ny forelder på 3. trinn").unwrap();
        assert_eq!(open(&sk, &aad, &sealed).unwrap().as_str(), "Hei, jeg er ny forelder på 3. trinn");
        let (_, other_sk) = generate_sealing_pair();
        assert_eq!(open(&other_sk, &aad, &sealed).unwrap_err(), CryptoError::Decrypt);
        let moved = Aad { row_id: Uuid::now_v7(), ..aad };
        assert_eq!(open(&sk, &moved, &sealed).unwrap_err(), CryptoError::Decrypt);
    }

    #[test]
    fn keys_round_trip_through_bytes() {
        let (pk, sk) = generate_sealing_pair();
        let aad = Aad::new(Uuid::now_v7(), "t", "c", Uuid::now_v7());
        let sealed = seal(&SealingPublicKey::from_bytes(pk.to_bytes()), &aad, "x").unwrap();
        assert_eq!(open(&SealingPrivateKey::from_bytes(*sk.expose()), &aad, &sealed).unwrap().as_str(), "x");
        assert_eq!(format!("{sk:?}"), "SealingPrivateKey([redacted])");
    }

    #[test]
    fn a_500_character_message_seals_within_2200_bytes() {
        let (pk, _) = generate_sealing_pair();
        let aad = Aad::new(Uuid::now_v7(), "t", "c", Uuid::now_v7());
        assert!(seal(&pk, &aad, &"ø".repeat(500)).unwrap().as_bytes().len() <= 2200);
    }
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p fau-crypto`
Expected: compile errors (`encrypt`, `Aad` etc. not defined).

- [ ] **Step 4: Implement**

`src/error.rs`:

```rust
use std::fmt;

/// Deliberately coarse: a caller learns that decryption failed, never why, and no
/// variant carries bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    Randomness,
    Malformed,
    Decrypt,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Randomness => "the operating system's random source failed",
            Self::Malformed => "the stored value is not a recognised envelope",
            Self::Decrypt => "the value could not be decrypted",
        })
    }
}

impl std::error::Error for CryptoError {}
```

`src/key.rs`:

```rust
use std::fmt;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::CryptoError;

/// A 256-bit data key handed out by the key service. Zeroised on drop; never printed.
#[derive(Clone)]
pub struct DataKey(Zeroizing<[u8; 32]>);

impl DataKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn generate() -> Result<Self, CryptoError> {
        let mut k = Zeroizing::new([0u8; 32]);
        getrandom::fill(k.as_mut()).map_err(|_| CryptoError::Randomness)?;
        Ok(Self(k))
    }

    /// Named so that every place key bytes are read is greppable.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataKey([redacted])")
    }
}

/// The key service's id for a key. Not secret; written inside every envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyId(Uuid);

impl KeyId {
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }
    pub fn uuid(self) -> Uuid {
        self.0
    }
}
```

`src/envelope.rs` (tests from Step 2 at the bottom):

```rust
//! `version (1) ‖ key_id (16) ‖ nonce (24) ‖ XChaCha20-Poly1305 ciphertext+tag`
//! (docs/key-service-design.md §5.1). The associated data binds a value to its FAU,
//! table, column and row, so it cannot be moved.

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::CryptoError;
use crate::key::{DataKey, KeyId};

const VERSION: u8 = 1;
const HEADER: usize = 1 + 16 + 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aad {
    pub tenant_id: Uuid,
    pub table: &'static str,
    pub column: &'static str,
    pub row_id: Uuid,
}

impl Aad {
    pub fn new(tenant_id: Uuid, table: &'static str, column: &'static str, row_id: Uuid) -> Self {
        Self { tenant_id, table, column, row_id }
    }

    /// Length-prefixed, so ("ab","c") and ("a","bc") cannot collide.
    pub(crate) fn bytes(&self) -> Vec<u8> {
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

/// Opaque stored ciphertext. Persistence functions for encrypted columns accept only
/// this type, so plaintext has no way into them.
#[derive(Clone, PartialEq, Eq)]
pub struct Ciphertext(Vec<u8>);

impl Ciphertext {
    /// For values read back from the database.
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

pub fn encrypt(key_id: KeyId, key: &DataKey, aad: &Aad, plaintext: &str) -> Result<Ciphertext, CryptoError> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| CryptoError::Randomness)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.expose()));
    let body = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext.as_bytes(), aad: &aad.bytes() })
        .map_err(|_| CryptoError::Decrypt)?;
    let mut out = Vec::with_capacity(HEADER + body.len());
    out.push(VERSION);
    out.extend_from_slice(key_id.uuid().as_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&body);
    Ok(Ciphertext(out))
}

pub fn key_id_of(ct: &Ciphertext) -> Result<KeyId, CryptoError> {
    let b = &ct.0;
    if b.len() < HEADER + 16 || b[0] != VERSION {
        return Err(CryptoError::Malformed);
    }
    let id = Uuid::from_slice(&b[1..17]).map_err(|_| CryptoError::Malformed)?;
    Ok(KeyId::new(id))
}

pub fn decrypt(key: &DataKey, aad: &Aad, ct: &Ciphertext) -> Result<Zeroizing<String>, CryptoError> {
    key_id_of(ct)?;
    let b = &ct.0;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.expose()));
    let plain = Zeroizing::new(
        cipher
            .decrypt(XNonce::from_slice(&b[17..HEADER]), Payload { msg: &b[HEADER..], aad: &aad.bytes() })
            .map_err(|_| CryptoError::Decrypt)?,
    );
    let text = std::str::from_utf8(&plain).map_err(|_| CryptoError::Malformed)?;
    Ok(Zeroizing::new(text.to_owned()))
}
```

`src/seal.rs` (tests from Step 2 at the bottom):

```rust
//! HPKE base mode, DHKEM(X25519, HKDF-SHA256) / HKDF-SHA256 / ChaCha20-Poly1305
//! (RFC 9180; docs/key-service-design.md §5.1). Stored as `enc (32) ‖ ciphertext`.
//! The backend can seal but never read back what it sealed.

use std::fmt;

use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};
use zeroize::Zeroizing;

use crate::envelope::Aad;
use crate::error::CryptoError;

const INFO: &[u8] = b"fau-sealed-v1";
const ENC_LEN: usize = 32;

type K = X25519HkdfSha256;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SealingPublicKey([u8; 32]);

impl SealingPublicKey {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }
    pub fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for SealingPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SealingPublicKey")
    }
}

pub struct SealingPrivateKey(Zeroizing<[u8; 32]>);

impl SealingPrivateKey {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(Zeroizing::new(b))
    }
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SealingPrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SealingPrivateKey([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Sealed(Vec<u8>);

impl Sealed {
    pub fn from_stored(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Sealed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sealed({} bytes)", self.0.len())
    }
}

pub fn generate_sealing_pair() -> (SealingPublicKey, SealingPrivateKey) {
    let (sk, pk) = K::gen_keypair(&mut rand_core::OsRng);
    let mut skb = [0u8; 32];
    skb.copy_from_slice(&sk.to_bytes());
    let mut pkb = [0u8; 32];
    pkb.copy_from_slice(&pk.to_bytes());
    (SealingPublicKey(pkb), SealingPrivateKey::from_bytes(skb))
}

pub fn seal(pk: &SealingPublicKey, aad: &Aad, plaintext: &str) -> Result<Sealed, CryptoError> {
    let pk = <K as Kem>::PublicKey::from_bytes(&pk.0).map_err(|_| CryptoError::Malformed)?;
    let (enc, ct) = hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, K, _>(
        &OpModeS::Base, &pk, INFO, plaintext.as_bytes(), &aad.bytes(), &mut rand_core::OsRng,
    )
    .map_err(|_| CryptoError::Randomness)?;
    let mut out = enc.to_bytes().to_vec();
    out.extend_from_slice(&ct);
    Ok(Sealed(out))
}

pub fn open(sk: &SealingPrivateKey, aad: &Aad, sealed: &Sealed) -> Result<Zeroizing<String>, CryptoError> {
    if sealed.0.len() <= ENC_LEN {
        return Err(CryptoError::Malformed);
    }
    let sk = <K as Kem>::PrivateKey::from_bytes(sk.expose()).map_err(|_| CryptoError::Malformed)?;
    let enc = <K as Kem>::EncappedKey::from_bytes(&sealed.0[..ENC_LEN]).map_err(|_| CryptoError::Malformed)?;
    let plain = Zeroizing::new(
        hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, K>(
            &OpModeR::Base, &sk, &enc, INFO, &sealed.0[ENC_LEN..], &aad.bytes(),
        )
        .map_err(|_| CryptoError::Decrypt)?,
    );
    let text = std::str::from_utf8(&plain).map_err(|_| CryptoError::Malformed)?;
    Ok(Zeroizing::new(text.to_owned()))
}
```

`src/lib.rs`:

```rust
//! Backend-side encryption for FAU (docs/key-service-design.md §5.1).

mod envelope;
mod error;
mod key;
mod seal;

pub use envelope::{decrypt, encrypt, key_id_of, Aad, Ciphertext};
pub use error::CryptoError;
pub use key::{DataKey, KeyId};
pub use seal::{generate_sealing_pair, open, seal, Sealed, SealingPrivateKey, SealingPublicKey};
pub use zeroize::Zeroizing;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p fau-crypto`
Expected: 9 passed.

- [ ] **Step 6: Commit**

```bash
git add backend/Cargo.toml backend/Cargo.lock backend/crates/crypto backend/crates/key-protocol backend/crates/key-service backend/crates/key-client
git commit -m "Add fau-crypto: the envelope, field cipher and HPKE sealing (#3506)"
```

---

### Task 2: `fau-key-protocol`, the wire types and the pinned route list

**Files:**
- Replace the stub: `backend/crates/key-protocol/Cargo.toml`, `src/lib.rs`
- Test: unit tests in `src/lib.rs`

**Interfaces:**
- Produces (used by Tasks 6–8):
  - `KeyClass { Kek, Record, Document, ChatEpoch, Sealing, Invitation }`, with `code() -> &'static str` and `from_code(&str) -> Option<KeyClass>`.
  - `Caller { account_id: Uuid, session_id: Uuid, instance_id: String }`.
  - `CreateFauRequest { fau_id, caller }`, `PublicKeyResponse { public_key: String }`.
  - `UnwrapRequest { fau_id, class, subject: Option<String>, caller }`, `CreateKeyRequest { fau_id, class, subject: String, caller }`.
  - `KeyResponse { key_id: Uuid, key: String, lease_id: Uuid, idle_timeout_secs: u64 }`.
  - `LeaseRequest { lease_id, instance_id }`, `LeaseResponse { idle_timeout_secs }`.
  - `DestroyKeyRequest { fau_id, class, subject: String, caller }`, `DestroyFauRequest { caller }`, `DestroyFauResponse { due_at: String }`.
  - `UnsealRequest { root_key: String }`.
  - `ErrorCode { Sealed, CeilingReached, RateLimited, NotFound, Refused, BadRequest, Internal }`, `ErrorBody { error: ErrorCode }`.
  - `encode_key(&[u8; 32]) -> String`, `decode_key(&str) -> Option<Zeroizing<[u8; 32]>>`.
  - `ROUTES: &[(&str, &str)]`.

- [ ] **Step 1: Write the failing tests** (bottom of `src/lib.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_round_trip_and_serialise_snake_case() {
        for c in KeyClass::ALL {
            assert_eq!(KeyClass::from_code(c.code()), Some(*c));
            assert_eq!(serde_json::to_string(c).unwrap(), format!("\"{}\"", c.code()));
        }
        assert_eq!(KeyClass::ChatEpoch.code(), "chat_epoch");
    }

    #[test]
    fn keys_round_trip_through_base64url() {
        let k = [0xabu8; 32];
        assert_eq!(*decode_key(&encode_key(&k)).unwrap(), k);
        assert!(decode_key("short").is_none());
    }

    #[test]
    fn secret_bearing_types_redact_debug() {
        let r = KeyResponse { key_id: Uuid::nil(), key: encode_key(&[1; 32]), lease_id: Uuid::nil(), idle_timeout_secs: 5 };
        assert!(!format!("{r:?}").contains(&r.key));
        let u = UnsealRequest { root_key: "SECRET".into() };
        assert!(!format!("{u:?}").contains("SECRET"));
    }

    #[test]
    fn the_route_list_has_no_duplicates_and_no_list_or_export_route() {
        let mut seen = std::collections::HashSet::new();
        for r in ROUTES {
            assert!(seen.insert(*r), "duplicate route {r:?}");
            let p = r.1.to_ascii_lowercase();
            for banned in ["list", "export", "bulk", "all", "dump", "kek", "root"] {
                assert!(!p.contains(banned), "route {r:?} looks like a bulk or key-exporting endpoint");
            }
        }
    }

    #[test]
    fn error_codes_serialise_snake_case() {
        assert_eq!(serde_json::to_string(&ErrorBody { error: ErrorCode::CeilingReached }).unwrap(), r#"{"error":"ceiling_reached"}"#);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-key-protocol`
Expected: compile errors.

- [ ] **Step 3: Implement**

`Cargo.toml` `[dependencies]`: `base64 = { workspace = true }`, `serde = { workspace = true }`, `uuid = { workspace = true }`, `zeroize = { workspace = true }`; `[dev-dependencies]`: `serde_json = { workspace = true }`.

`src/lib.rs`:

```rust
//! The key service's wire contract (docs/key-service-design.md §3.2), shared by the
//! server (fau-key-service) and the client (fau-key-client). Serde types only.

use std::fmt;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyClass {
    Kek,
    Record,
    Document,
    ChatEpoch,
    Sealing,
    Invitation,
}

impl KeyClass {
    pub const ALL: &'static [KeyClass] = &[
        KeyClass::Kek, KeyClass::Record, KeyClass::Document,
        KeyClass::ChatEpoch, KeyClass::Sealing, KeyClass::Invitation,
    ];

    pub fn code(self) -> &'static str {
        match self {
            KeyClass::Kek => "kek",
            KeyClass::Record => "record",
            KeyClass::Document => "document",
            KeyClass::ChatEpoch => "chat_epoch",
            KeyClass::Sealing => "sealing",
            KeyClass::Invitation => "invitation",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.code() == code)
    }
}

/// Who the backend says the key is for. The key service does not verify it against
/// memberships (spec K3); it logs it and rate-limits by it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caller {
    pub account_id: Uuid,
    pub session_id: Uuid,
    /// The backend pod's name. Combined with the client certificate's CN for the ceiling.
    pub instance_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFauRequest { pub fau_id: Uuid, pub caller: Caller }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicKeyResponse { pub public_key: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnwrapRequest {
    pub fau_id: Uuid,
    pub class: KeyClass,
    /// Document or invitation id, or `YYYY-MM` for a chat epoch; `None` for record and sealing.
    pub subject: Option<String>,
    pub caller: Caller,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateKeyRequest { pub fau_id: Uuid, pub class: KeyClass, pub subject: String, pub caller: Caller }

#[derive(Clone, Serialize, Deserialize)]
pub struct KeyResponse {
    pub key_id: Uuid,
    /// base64url, no padding. Decode with [`decode_key`] straight into zeroising memory.
    pub key: String,
    pub lease_id: Uuid,
    pub idle_timeout_secs: u64,
}

impl fmt::Debug for KeyResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyResponse")
            .field("key_id", &self.key_id)
            .field("key", &"[redacted]")
            .field("lease_id", &self.lease_id)
            .field("idle_timeout_secs", &self.idle_timeout_secs)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRequest { pub lease_id: Uuid, pub instance_id: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseResponse { pub idle_timeout_secs: u64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyKeyRequest { pub fau_id: Uuid, pub class: KeyClass, pub subject: String, pub caller: Caller }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyFauRequest { pub caller: Caller }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyFauResponse { pub due_at: String }

#[derive(Clone, Serialize, Deserialize)]
pub struct UnsealRequest { pub root_key: String }

impl fmt::Debug for UnsealRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UnsealRequest([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode { Sealed, CeilingReached, RateLimited, NotFound, Refused, BadRequest, Internal }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody { pub error: ErrorCode }

pub fn encode_key(key: &[u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

pub fn decode_key(s: &str) -> Option<Zeroizing<[u8; 32]>> {
    let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(s).ok()?);
    let mut out = Zeroizing::new([0u8; 32]);
    if bytes.len() != 32 {
        return None;
    }
    out.copy_from_slice(&bytes);
    Some(out)
}

/// Every route the API listener serves. `tests/api.rs` asserts that the router serves
/// exactly these, so adding a route means editing this list, where reviewers look.
pub const ROUTES: &[(&str, &str)] = &[
    ("GET", "/health/live"),
    ("GET", "/health/ready"),
    ("POST", "/v1/faus"),
    ("GET", "/v1/faus/{fau_id}/public-key"),
    ("POST", "/v1/faus/{fau_id}/destroy"),
    ("POST", "/v1/keys/create"),
    ("POST", "/v1/keys/unwrap"),
    ("POST", "/v1/keys/destroy"),
    ("POST", "/v1/leases/renew"),
    ("POST", "/v1/leases/release"),
];
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p fau-key-protocol`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/key-protocol backend/Cargo.lock
git commit -m "Add fau-key-protocol: the key service's wire types and pinned routes (#3506)"
```

---

### Task 3: The key service store: root key, wrapping and SQLite

**Files:**
- Replace the stub: `backend/crates/key-service/Cargo.toml`
- Create: `backend/crates/key-service/migrations/0001_store.sql`
- Create: `src/lib.rs`, `src/root.rs`, `src/wrap.rs`, `src/store.rs`
- Test: unit tests in `root.rs` and `wrap.rs`; async tests in `store.rs`

**Interfaces:**
- Consumes: `fau_key_protocol::KeyClass`.
- Produces:
  - `RootKey`: `generate()`, `encode() -> Zeroizing<String>`, `decode(&str) -> Result<RootKey, RootKeyError>`, `expose() -> &[u8; 32]`.
  - `wrap::seal_key(wrapping: &[u8; 32], aad: &[u8], key: &[u8; 32]) -> (Vec<u8> /*nonce*/, Vec<u8> /*ct*/)`.
  - `wrap::open_key(wrapping: &[u8; 32], aad: &[u8], nonce: &[u8], ct: &[u8]) -> Option<Zeroizing<[u8; 32]>>`.
  - `Store::open(path: &Path) -> Result<Store, StoreError>`.
  - `Store::has_canary()`, `write_canary(&RootKey)`, `check_canary(&RootKey) -> Result<bool, StoreError>`.
  - `Store::create_fau(&RootKey, fau: Uuid) -> Result<[u8; 32] /*public key*/, StoreError>`, idempotent.
  - `Store::public_key(fau) -> Result<Option<[u8; 32]>, StoreError>`.
  - `Store::create_key(&RootKey, fau, class, subject: &str) -> Result<(Uuid, Zeroizing<[u8; 32]>), StoreError>`, which returns the existing key if it is already present.
  - `Store::unwrap_key(&RootKey, fau, class, subject: Option<&str>) -> Result<Option<(Uuid, Zeroizing<[u8; 32]>)>, StoreError>`.
  - `Store::destroy_key(fau, class, subject: &str) -> Result<bool, StoreError>`.
  - `Store::destroy_fau(fau, now: Timestamp) -> Result<Option<Timestamp> /*due_at*/, StoreError>`.
  - `StoreError { Sqlite, FauUnknown, Corrupt }`.

- [ ] **Step 1: Write the Cargo manifest and migration**

`backend/crates/key-service/Cargo.toml`:

```toml
[package]
name = "fau-key-service"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

[[bin]]
name = "fau-keys"
path = "src/main.rs"

[features]
# Honours FAU_KEYS_DEV_ROOT_KEY. Never enabled in the release image (spec §6.4).
dev = []
# Exposes `test_support` for fau-key-client's and fau-app's integration tests. Only ever
# enabled from a dev-dependency, so resolver 2 keeps it out of release builds.
test-support = []

[dependencies]
fau-key-protocol = { path = "../key-protocol" }
axum = { workspace = true }
base64 = { workspace = true }
chacha20poly1305 = { workspace = true }
clap = { workspace = true }
getrandom = { workspace = true }
hpke = { workspace = true }
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["tokio", "server-auto", "service"] }
jiff = { workspace = true }
libc = "0.2"
rand_core = { workspace = true }
rcgen = "0.14"
rpassword = "7"
rustls = { version = "0.23", default-features = false, features = ["ring", "std"] }
rustls-pki-types = "1"
serde = { workspace = true }
serde_json = { workspace = true }
sha2 = { workspace = true }
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "sqlite", "macros", "migrate"] }
thiserror = { workspace = true }
tokio = { workspace = true }
tokio-rustls = { version = "0.26", default-features = false, features = ["ring"] }
tower = { version = "0.5", features = ["util"] }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
uuid = { workspace = true }
x509-parser = "0.18"
zeroize = { workspace = true }
reqwest = { version = "0.13", default-features = false, features = ["rustls", "json"] }

[dev-dependencies]
tempfile = "3"
tokio = { workspace = true, features = ["io-util"] }
```

(`reqwest` is a normal dependency because `fau-keys unseal` is an HTTP client. The `rustls` in
`sqlx` is not used here: the `sqlite` driver needs no TLS.)

`migrations/0001_store.sql`:

```sql
-- The key service's own store (docs/key-service-design.md §4.1). SQLite, one writer,
-- secure_delete on, rollback journal. Never snapshotted or backed up.

create table canary (
  id          integer primary key check (id = 1),
  nonce       blob not null,
  ciphertext  blob not null
);

create table keys (
  id          text not null primary key,   -- uuid
  fau_id      text not null,
  class       text not null check (class in ('kek','record','document','chat_epoch','sealing','invitation')),
  subject     text,                        -- document/invitation id, YYYY-MM, or null
  created_at  text not null,
  unique (fau_id, class, subject)
);
-- SQLite treats NULLs as distinct in UNIQUE; this index makes (fau, class) unique for
-- the per-FAU classes, whose subject is null.
create unique index keys_per_fau_singletons on keys (fau_id, class) where subject is null;

create table wraps (
  key_id      text not null references keys (id) on delete cascade,
  kind        text not null check (kind in ('root','kek')),
  nonce       blob not null,
  ciphertext  blob not null,
  primary key (key_id, kind)
);

create table public_keys (
  fau_id      text not null primary key,
  public_key  blob not null
);

create table destruction_queue (
  key_id        text not null primary key,
  fau_id        text not null,
  enqueued_at   text not null,
  due_at        text not null,
  cancelled_at  text
);
```

- [ ] **Step 2: Write the failing tests**

`src/root.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_with_a_checksum_that_catches_typos() {
        let k = RootKey::generate().unwrap();
        let s = k.encode();
        assert_eq!(s.len(), 43 + 1 + 4);
        assert_eq!(RootKey::decode(&s).unwrap().expose(), k.expose());
        let mut typo: Vec<char> = s.chars().collect();
        typo[3] = if typo[3] == 'A' { 'B' } else { 'A' };
        let typo: String = typo.into_iter().collect();
        assert_eq!(RootKey::decode(&typo).unwrap_err(), RootKeyError::Checksum);
        assert_eq!(RootKey::decode("nonsense").unwrap_err(), RootKeyError::Format);
        assert_eq!(RootKey::decode(&format!("  {}\n", &*s)).unwrap().expose(), k.expose(), "surrounding whitespace from a paste is tolerated");
    }

    #[test]
    fn debug_is_redacted() {
        assert_eq!(format!("{:?}", RootKey::generate().unwrap()), "RootKey([redacted])");
    }
}
```

`src/wrap.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_refuses_the_wrong_key_or_aad() {
        let w = [1u8; 32];
        let k = [2u8; 32];
        let (n, ct) = seal_key(&w, b"aad", &k).unwrap();
        assert_eq!(*open_key(&w, b"aad", &n, &ct).unwrap(), k);
        assert!(open_key(&[3u8; 32], b"aad", &n, &ct).is_none());
        assert!(open_key(&w, b"other", &n, &ct).is_none());
    }
}
```

`src/store.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use fau_key_protocol::KeyClass;

    async fn store() -> (tempfile::TempDir, Store, RootKey) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("keys.db")).await.unwrap();
        let root = RootKey::generate().unwrap();
        s.write_canary(&root).await.unwrap();
        (dir, s, root)
    }

    #[tokio::test]
    async fn the_canary_accepts_only_its_root_key() {
        let (_d, s, root) = store().await;
        assert!(s.has_canary().await.unwrap());
        assert!(s.check_canary(&root).await.unwrap());
        assert!(!s.check_canary(&RootKey::generate().unwrap()).await.unwrap());
    }

    #[tokio::test]
    async fn every_class_round_trips() {
        let (_d, s, root) = store().await;
        let fau = Uuid::now_v7();
        let pk = s.create_fau(&root, fau).await.unwrap();
        assert_eq!(s.create_fau(&root, fau).await.unwrap(), pk, "idempotent");
        assert_eq!(s.public_key(fau).await.unwrap(), Some(pk));
        for class in [KeyClass::Record, KeyClass::Sealing] {
            assert!(s.unwrap_key(&root, fau, class, None).await.unwrap().is_some(), "{class:?}");
        }
        for (class, subject) in [(KeyClass::Document, Uuid::now_v7().to_string()), (KeyClass::Invitation, Uuid::now_v7().to_string()), (KeyClass::ChatEpoch, "2026-09".to_string())] {
            let (id, k) = s.create_key(&root, fau, class, &subject).await.unwrap();
            let (id2, k2) = s.unwrap_key(&root, fau, class, Some(&subject)).await.unwrap().unwrap();
            assert_eq!((id, *k), (id2, *k2), "{class:?}");
            let (id3, _) = s.create_key(&root, fau, class, &subject).await.unwrap();
            assert_eq!(id, id3, "create is idempotent per subject");
        }
    }

    #[tokio::test]
    async fn the_kek_is_never_unwrappable_through_the_store_api() {
        let (_d, s, root) = store().await;
        let fau = Uuid::now_v7();
        s.create_fau(&root, fau).await.unwrap();
        assert!(s.unwrap_key(&root, fau, KeyClass::Kek, None).await.is_err());
    }

    #[tokio::test]
    async fn a_destroyed_key_is_gone_from_the_file_bytes() {
        let (dir, s, root) = store().await;
        let fau = Uuid::now_v7();
        s.create_fau(&root, fau).await.unwrap();
        let doc = Uuid::now_v7().to_string();
        let (key_id, _) = s.create_key(&root, fau, KeyClass::Document, &doc).await.unwrap();
        let wrapped = s.raw_wrap_for_tests(key_id).await.unwrap();
        let file = dir.path().join("keys.db");
        let contains = |needle: &[u8]| std::fs::read(&file).unwrap().windows(needle.len()).any(|w| w == needle);
        assert!(contains(&wrapped), "the wrapped key is on disk before destruction");
        assert!(s.destroy_key(fau, KeyClass::Document, &doc).await.unwrap());
        assert!(!contains(&wrapped), "secure_delete must overwrite the freed page");
        assert!(s.unwrap_key(&root, fau, KeyClass::Document, Some(&doc)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn destroying_an_fau_removes_the_kek_and_queues_it_for_seven_days() {
        let (_d, s, root) = store().await;
        let fau = Uuid::now_v7();
        s.create_fau(&root, fau).await.unwrap();
        let now: jiff::Timestamp = "2026-09-27T10:00:00Z".parse().unwrap();
        let due = s.destroy_fau(fau, now).await.unwrap().unwrap();
        assert_eq!(due, now + jiff::SignedDuration::from_hours(7 * 24));
        assert!(s.unwrap_key(&root, fau, KeyClass::Record, None).await.is_err(), "no KEK, nothing below it opens");
        assert_eq!(s.queue_len_for_tests().await.unwrap(), 1);
        assert_eq!(s.destroy_fau(fau, now).await.unwrap(), None, "an already destroyed FAU is not queued twice");
    }

    #[tokio::test]
    async fn the_pragmas_hold() {
        let (_d, s, _) = store().await;
        assert_eq!(s.pragmas_for_tests().await.unwrap(), ("delete".to_string(), 1, 2));
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p fau-key-service --lib`
Expected: compile errors.

- [ ] **Step 4: Implement**

`src/root.rs`:

```rust
//! The root key (spec §4.3). 32 bytes, printed once at `init` as base64url plus a
//! 4-hex-character SHA-256 checksum, so a mistyped paste is caught before the canary.

use std::fmt;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKeyError { Format, Checksum, Randomness }

pub struct RootKey(Zeroizing<[u8; 32]>);

impl RootKey {
    pub fn generate() -> Result<Self, RootKeyError> {
        let mut k = Zeroizing::new([0u8; 32]);
        getrandom::fill(k.as_mut()).map_err(|_| RootKeyError::Randomness)?;
        Ok(Self(k))
    }

    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }

    fn checksum(k: &[u8; 32]) -> String {
        let d = Sha256::digest(k);
        format!("{:02x}{:02x}", d[0], d[1])
    }

    pub fn encode(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("{}-{}", URL_SAFE_NO_PAD.encode(*self.0), Self::checksum(&self.0)))
    }

    pub fn decode(s: &str) -> Result<Self, RootKeyError> {
        let s = s.trim();
        let (body, sum) = s.rsplit_once('-').ok_or(RootKeyError::Format)?;
        if body.len() != 43 || sum.len() != 4 {
            return Err(RootKeyError::Format);
        }
        let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(body).map_err(|_| RootKeyError::Format)?);
        if bytes.len() != 32 {
            return Err(RootKeyError::Format);
        }
        let mut k = Zeroizing::new([0u8; 32]);
        k.copy_from_slice(&bytes);
        if Self::checksum(&k) != sum {
            return Err(RootKeyError::Checksum);
        }
        Ok(Self(k))
    }
}

impl fmt::Debug for RootKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RootKey([redacted])")
    }
}
```

Note: base64url's alphabet includes `-`, so the body may itself contain `-`. `rsplit_once('-')`
splits at the last one, and the checksum is fixed at 4 hex characters, so this is unambiguous.

`src/wrap.rs`:

```rust
//! XChaCha20-Poly1305 key wrapping. The associated data names the wrapped key's id,
//! class, FAU and wrap kind, so a wrap cannot be moved onto another row (spec §3.1).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

pub fn seal_key(wrapping: &[u8; 32], aad: &[u8], key: &[u8; 32]) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).ok()?;
    let ct = XChaCha20Poly1305::new(Key::from_slice(wrapping))
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: key, aad })
        .ok()?;
    Some((nonce.to_vec(), ct))
}

pub fn open_key(wrapping: &[u8; 32], aad: &[u8], nonce: &[u8], ct: &[u8]) -> Option<Zeroizing<[u8; 32]>> {
    if nonce.len() != 24 {
        return None;
    }
    let plain = Zeroizing::new(
        XChaCha20Poly1305::new(Key::from_slice(wrapping))
            .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
            .ok()?,
    );
    if plain.len() != 32 {
        return None;
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&plain);
    Some(out)
}

pub fn aad(key_id: &str, class: &str, fau_id: &str, kind: &str) -> Vec<u8> {
    format!("fau-wrap-v1|{key_id}|{class}|{fau_id}|{kind}").into_bytes()
}
```

(The Interfaces block above shows `seal_key` returning a tuple; it returns `Option<(..)>`, and
`None` means the random source failed. Callers map `None` to `StoreError::Corrupt`.)

`src/store.rs`, the whole file apart from the tests:

```rust
//! The SQLite key store (spec §4.1). Every stored key is wrapped: a KEK under the
//! root key (`kind = 'root'`), every other key under its FAU's KEK (`kind = 'kek'`).

use std::path::Path;
use std::str::FromStr;

use fau_key_protocol::KeyClass;
use jiff::{SignedDuration, Timestamp};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::root::RootKey;
use crate::wrap::{aad, open_key, seal_key};

const CANARY: &[u8] = b"fau-keys canary v1";
const CANARY_AAD: &[u8] = b"fau-keys-canary";
/// ADR-003 decision 7.
pub const DESTRUCTION_WINDOW: SignedDuration = SignedDuration::from_hours(7 * 24);

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the key store could not be read or written")]
    Sqlite(#[from] sqlx::Error),
    #[error("no such FAU in the key store")]
    FauUnknown,
    #[error("a stored key failed to open")]
    Corrupt,
}

pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Delete)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .pragma("secure_delete", "ON");
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?;
        sqlx::migrate!("./migrations").run(&pool).await.map_err(sqlx::Error::from)?;
        Ok(Self { pool })
    }

    pub async fn has_canary(&self) -> Result<bool, StoreError> {
        Ok(sqlx::query_scalar::<_, i64>("select count(*) from canary").fetch_one(&self.pool).await? == 1)
    }

    pub async fn write_canary(&self, root: &RootKey) -> Result<(), StoreError> {
        let mut padded = [0u8; 32];
        padded[..CANARY.len()].copy_from_slice(CANARY);
        let (nonce, ct) = seal_key(root.expose(), CANARY_AAD, &padded).ok_or(StoreError::Corrupt)?;
        sqlx::query("insert into canary (id, nonce, ciphertext) values (1, ?, ?)")
            .bind(nonce).bind(ct).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn check_canary(&self, root: &RootKey) -> Result<bool, StoreError> {
        let row: Option<(Vec<u8>, Vec<u8>)> = sqlx::query_as("select nonce, ciphertext from canary where id = 1")
            .fetch_optional(&self.pool).await?;
        let Some((nonce, ct)) = row else { return Ok(false) };
        Ok(open_key(root.expose(), CANARY_AAD, &nonce, &ct).is_some_and(|p| p[..CANARY.len()] == *CANARY))
    }

    async fn insert_key(
        &self,
        tx: &mut sqlx::SqliteConnection,
        wrapping: &[u8; 32],
        fau: Uuid,
        class: KeyClass,
        subject: Option<&str>,
        kind: &str,
        key: &[u8; 32],
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();
        let (fau_s, id_s) = (fau.to_string(), id.to_string());
        sqlx::query("insert into keys (id, fau_id, class, subject, created_at) values (?, ?, ?, ?, ?)")
            .bind(&id_s).bind(&fau_s).bind(class.code()).bind(subject).bind(Timestamp::now().to_string())
            .execute(&mut *tx).await?;
        let (nonce, ct) = seal_key(wrapping, &aad(&id_s, class.code(), &fau_s, kind), key).ok_or(StoreError::Corrupt)?;
        sqlx::query("insert into wraps (key_id, kind, nonce, ciphertext) values (?, ?, ?, ?)")
            .bind(&id_s).bind(kind).bind(nonce).bind(ct).execute(&mut *tx).await?;
        Ok(id)
    }

    fn random_key() -> Result<Zeroizing<[u8; 32]>, StoreError> {
        let mut k = Zeroizing::new([0u8; 32]);
        getrandom::fill(k.as_mut()).map_err(|_| StoreError::Corrupt)?;
        Ok(k)
    }

    /// Creates the KEK, the record key and the sealing pair. Idempotent.
    pub async fn create_fau(&self, root: &RootKey, fau: Uuid) -> Result<[u8; 32], StoreError> {
        if let Some(pk) = self.public_key(fau).await? {
            return Ok(pk);
        }
        let mut tx = self.pool.begin().await?;
        let kek = Self::random_key()?;
        self.insert_key(&mut tx, root.expose(), fau, KeyClass::Kek, None, "root", &kek).await?;
        let record = Self::random_key()?;
        self.insert_key(&mut tx, &kek, fau, KeyClass::Record, None, "kek", &record).await?;
        let (sk, pk) = {
            use hpke::{kem::X25519HkdfSha256, Kem, Serializable};
            let (sk, pk) = X25519HkdfSha256::gen_keypair(&mut rand_core::OsRng);
            let mut skb = Zeroizing::new([0u8; 32]);
            skb.copy_from_slice(&sk.to_bytes());
            let mut pkb = [0u8; 32];
            pkb.copy_from_slice(&pk.to_bytes());
            (skb, pkb)
        };
        self.insert_key(&mut tx, &kek, fau, KeyClass::Sealing, None, "kek", &sk).await?;
        sqlx::query("insert into public_keys (fau_id, public_key) values (?, ?)")
            .bind(fau.to_string()).bind(pk.to_vec()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(pk)
    }

    pub async fn public_key(&self, fau: Uuid) -> Result<Option<[u8; 32]>, StoreError> {
        let row: Option<Vec<u8>> = sqlx::query_scalar("select public_key from public_keys where fau_id = ?")
            .bind(fau.to_string()).fetch_optional(&self.pool).await?;
        row.map(|b| b.try_into().map_err(|_| StoreError::Corrupt)).transpose()
    }

    async fn load(&self, fau: Uuid, class: KeyClass, subject: Option<&str>, kind: &str)
        -> Result<Option<(String, Vec<u8>, Vec<u8>)>, StoreError>
    {
        let sql = "select k.id, w.nonce, w.ciphertext from keys k join wraps w on w.key_id = k.id
                    where k.fau_id = ? and k.class = ? and k.subject is ? and w.kind = ?";
        Ok(sqlx::query_as(sql).bind(fau.to_string()).bind(class.code()).bind(subject).bind(kind)
            .fetch_optional(&self.pool).await?)
    }

    async fn kek(&self, root: &RootKey, fau: Uuid) -> Result<Zeroizing<[u8; 32]>, StoreError> {
        let (id, nonce, ct) = self.load(fau, KeyClass::Kek, None, "root").await?.ok_or(StoreError::FauUnknown)?;
        open_key(root.expose(), &aad(&id, KeyClass::Kek.code(), &fau.to_string(), "root"), &nonce, &ct)
            .ok_or(StoreError::Corrupt)
    }

    pub async fn unwrap_key(&self, root: &RootKey, fau: Uuid, class: KeyClass, subject: Option<&str>)
        -> Result<Option<(Uuid, Zeroizing<[u8; 32]>)>, StoreError>
    {
        if class == KeyClass::Kek {
            return Err(StoreError::Corrupt);
        }
        let kek = self.kek(root, fau).await?;
        let Some((id, nonce, ct)) = self.load(fau, class, subject, "kek").await? else { return Ok(None) };
        let key = open_key(&kek, &aad(&id, class.code(), &fau.to_string(), "kek"), &nonce, &ct).ok_or(StoreError::Corrupt)?;
        Ok(Some((Uuid::from_str(&id).map_err(|_| StoreError::Corrupt)?, key)))
    }

    pub async fn create_key(&self, root: &RootKey, fau: Uuid, class: KeyClass, subject: &str)
        -> Result<(Uuid, Zeroizing<[u8; 32]>), StoreError>
    {
        if let Some(found) = self.unwrap_key(root, fau, class, Some(subject)).await? {
            return Ok(found);
        }
        let kek = self.kek(root, fau).await?;
        let key = Self::random_key()?;
        let mut tx = self.pool.begin().await?;
        let id = self.insert_key(&mut tx, &kek, fau, class, Some(subject), "kek", &key).await?;
        tx.commit().await?;
        Ok((id, key))
    }

    pub async fn destroy_key(&self, fau: Uuid, class: KeyClass, subject: &str) -> Result<bool, StoreError> {
        let n = sqlx::query("delete from keys where fau_id = ? and class = ? and subject = ?")
            .bind(fau.to_string()).bind(class.code()).bind(subject)
            .execute(&self.pool).await?.rows_affected();
        Ok(n > 0)
    }

    /// Removes the live KEK at once and queues the replica-side removal (card 2).
    /// The keys under it stay: during the window a KEK recovered from the replica must
    /// still find them. Returns `None` if the FAU has no live KEK.
    pub async fn destroy_fau(&self, fau: Uuid, now: Timestamp) -> Result<Option<Timestamp>, StoreError> {
        let mut tx = self.pool.begin().await?;
        let id: Option<String> = sqlx::query_scalar("select id from keys where fau_id = ? and class = 'kek'")
            .bind(fau.to_string()).fetch_optional(&mut *tx).await?;
        let Some(id) = id else { return Ok(None) };
        let due = now + DESTRUCTION_WINDOW;
        sqlx::query("delete from keys where id = ?").bind(&id).execute(&mut *tx).await?;
        sqlx::query("insert into destruction_queue (key_id, fau_id, enqueued_at, due_at) values (?, ?, ?, ?)")
            .bind(&id).bind(fau.to_string()).bind(now.to_string()).bind(due.to_string())
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(due))
    }

    #[cfg(test)]
    pub(crate) async fn raw_wrap_for_tests(&self, key_id: Uuid) -> Result<Vec<u8>, StoreError> {
        Ok(sqlx::query_scalar("select ciphertext from wraps where key_id = ?").bind(key_id.to_string()).fetch_one(&self.pool).await?)
    }

    #[cfg(test)]
    pub(crate) async fn queue_len_for_tests(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("select count(*) from destruction_queue").fetch_one(&self.pool).await?)
    }

    #[cfg(test)]
    pub(crate) async fn pragmas_for_tests(&self) -> Result<(String, i64, i64), StoreError> {
        let jm: String = sqlx::query_scalar("pragma journal_mode").fetch_one(&self.pool).await?;
        let sd: i64 = sqlx::query_scalar("pragma secure_delete").fetch_one(&self.pool).await?;
        let sy: i64 = sqlx::query_scalar("pragma synchronous").fetch_one(&self.pool).await?;
        Ok((jm, sd, sy))
    }
}
```

`synchronous = FULL` reads back as `2`. `k.subject is ?` is SQLite's null-safe equality, which
the per-FAU classes need. `destroy_key` deletes from `keys`, and the `on delete cascade` removes
the wrap. The `foreign_keys(true)` option is what enables the cascade.

`src/lib.rs` for now:

```rust
//! The FAU key service (docs/key-service-design.md). A separate trust boundary: keep
//! it small enough to review in an afternoon (ADR-003 decision 5).

pub mod root;
pub mod store;
pub mod wrap;
```

`src/main.rs`, a placeholder so the declared `[[bin]]` builds until Task 7 replaces it:

```rust
fn main() {}
```

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test -p fau-key-service --lib`
Expected: 9 passed. If `a_destroyed_key_is_gone_from_the_file_bytes` fails, check that the
`secure_delete` pragma is set on the connection. `the_pragmas_hold` tells you which pragma is
wrong.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/key-service backend/Cargo.lock
git commit -m "Add the key service's SQLite store, root key and key wrapping (#3506)"
```

---

### Task 4: Vault, chat-epoch rules, leases and rate limits

These are pure in-memory logic with an injected clock and no I/O beyond the store. They are what
the API in Task 5 enforces.

**Files:**
- Create: `backend/crates/key-service/src/{vault,epoch,leases,limits}.rs`
- Modify: `src/lib.rs` (module list)
- Test: unit tests in each file

**Interfaces:**
- Consumes: `Store`, `RootKey` (Task 3).
- Produces:
  - `Vault::sealed(store: Arc<Store>) -> Vault`.
  - `Vault::unseal(&self, encoded: &str) -> Result<(), UnsealError>`.
  - `Vault::root(&self) -> Option<Arc<RootKey>>`.
  - `Vault::is_sealed(&self) -> bool`.
  - `UnsealError { BadFormat, WrongKey, NotInitialised, Store }`.
  - `initialise(store: &Store) -> Result<Zeroizing<String>, InitError>`, with `InitError { AlreadyInitialised, Store }`.
  - `epoch::current_month(now: Timestamp) -> String` (`YYYY-MM`, Europe/Oslo).
  - `epoch::classify(month: &str, now: Timestamp) -> Option<Epoch>`, with `Epoch { Past, Current, Future }`.
  - `epoch::may_destroy(month: &str, now: Timestamp) -> bool`.
  - `LeaseTable::new(idle: SignedDuration, ceiling: usize)`.
  - `.grant(instance: &str, fau: Uuid, now) -> Result<Grant, LeaseError>`, with `Grant { lease_id: Uuid, new_fau: bool }`.
  - `.renew(lease: Uuid, instance: &str, now) -> Result<(), LeaseError>`.
  - `.release(lease: Uuid, instance: &str)`.
  - `.live(now) -> usize`.
  - `LeaseError { CeilingReached, Unknown }`.
  - `RateLimiter::new(per_minute: u32)`, `.allow(key: &str, now: Timestamp) -> bool`.

- [ ] **Step 1: Write the failing tests**

`src/vault.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh() -> (tempfile::TempDir, Arc<Store>) {
        let d = tempfile::tempdir().unwrap();
        let s = Arc::new(Store::open(&d.path().join("k.db")).await.unwrap());
        (d, s)
    }

    #[tokio::test]
    async fn starts_sealed_and_opens_only_with_the_initialised_key() {
        let (_d, store) = fresh().await;
        let encoded = initialise(&store).await.unwrap();
        let v = Vault::sealed(store.clone());
        assert!(v.is_sealed());
        assert!(v.root().is_none());
        let other = RootKey::generate().unwrap().encode();
        assert_eq!(v.unseal(&other).await.unwrap_err(), UnsealError::WrongKey);
        assert!(v.is_sealed());
        assert_eq!(v.unseal("garbage").await.unwrap_err(), UnsealError::BadFormat);
        v.unseal(&encoded).await.unwrap();
        assert!(!v.is_sealed());
    }

    #[tokio::test]
    async fn init_refuses_an_initialised_store_and_unseal_refuses_an_empty_one() {
        let (_d, store) = fresh().await;
        let v = Vault::sealed(store.clone());
        assert_eq!(v.unseal(&RootKey::generate().unwrap().encode()).await.unwrap_err(), UnsealError::NotInitialised);
        initialise(&store).await.unwrap();
        assert_eq!(initialise(&store).await.unwrap_err(), InitError::AlreadyInitialised);
    }
}
```

`src/epoch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp { s.parse().unwrap() }

    #[test]
    fn the_month_is_oslo_time_not_utc() {
        // 22:30 UTC on 30 September is 00:30 on 1 October in Oslo (CEST, UTC+2).
        assert_eq!(current_month(ts("2026-09-30T22:30:00Z")), "2026-10");
        assert_eq!(current_month(ts("2026-09-30T21:59:59Z")), "2026-09");
    }

    #[test]
    fn classifies_past_current_and_future() {
        let now = ts("2026-09-27T12:00:00Z");
        assert_eq!(classify("2026-09", now), Some(Epoch::Current));
        assert_eq!(classify("2026-08", now), Some(Epoch::Past));
        assert_eq!(classify("2026-10", now), Some(Epoch::Future));
        assert_eq!(classify("2026-13", now), None);
        assert_eq!(classify("26-09", now), None);
    }

    #[test]
    fn an_epoch_may_be_destroyed_only_12_months_after_it_ended() {
        // September 2025 ends at 2025-10-01 00:00 Oslo = 2025-09-30T22:00Z.
        assert!(!may_destroy("2025-09", ts("2026-09-30T21:59:59Z")));
        assert!(may_destroy("2025-09", ts("2026-09-30T22:00:00Z")));
        assert!(!may_destroy("2026-09", ts("2026-09-27T12:00:00Z")));
        assert!(!may_destroy("nonsense", ts("2030-01-01T00:00:00Z")));
    }
}
```

`src/leases.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn t(mins: i64) -> Timestamp {
        "2026-09-27T12:00:00Z".parse::<Timestamp>().unwrap() + SignedDuration::from_mins(mins)
    }

    #[test]
    fn the_ceiling_counts_distinct_faus_per_instance() {
        let l = LeaseTable::new(SignedDuration::from_mins(30), 2);
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert!(l.grant("app-1", a, t(0)).unwrap().new_fau);
        assert!(!l.grant("app-1", a, t(0)).unwrap().new_fau, "a second lease on the same FAU is not new");
        l.grant("app-1", b, t(0)).unwrap();
        assert_eq!(l.grant("app-1", c, t(0)).unwrap_err(), LeaseError::CeilingReached);
        assert!(l.grant("app-2", c, t(0)).is_ok(), "the ceiling is per instance");
    }

    #[test]
    fn release_frees_a_slot_and_is_not_activity() {
        let l = LeaseTable::new(SignedDuration::from_mins(30), 1);
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let g = l.grant("app-1", a, t(0)).unwrap();
        l.release(g.lease_id, "app-1");
        assert_eq!(l.renew(g.lease_id, "app-1", t(1)).unwrap_err(), LeaseError::Unknown, "a released lease cannot be revived");
        assert!(l.grant("app-1", b, t(1)).is_ok());
    }

    #[test]
    fn an_idle_lease_expires_and_renewal_extends_it() {
        let l = LeaseTable::new(SignedDuration::from_mins(30), 1);
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let g = l.grant("app-1", a, t(0)).unwrap();
        l.renew(g.lease_id, "app-1", t(29)).unwrap();
        assert_eq!(l.grant("app-1", b, t(58)).unwrap_err(), LeaseError::CeilingReached, "renewed at 29, still live at 58");
        assert_eq!(l.renew(g.lease_id, "app-1", t(60)).unwrap_err(), LeaseError::Unknown, "idle since 29, expired at 59");
        assert!(l.grant("app-1", b, t(60)).is_ok());
        assert_eq!(l.live(t(60)), 1);
    }

    #[test]
    fn another_instance_cannot_renew_or_release_your_lease() {
        let l = LeaseTable::new(SignedDuration::from_mins(30), 5);
        let g = l.grant("app-1", Uuid::now_v7(), t(0)).unwrap();
        assert_eq!(l.renew(g.lease_id, "app-2", t(1)).unwrap_err(), LeaseError::Unknown);
        l.release(g.lease_id, "app-2");
        assert!(l.renew(g.lease_id, "app-1", t(2)).is_ok());
    }
}
```

`src/limits.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_the_budget_then_refuses_then_refills() {
        let r = RateLimiter::new(3);
        let t0: Timestamp = "2026-09-27T12:00:00Z".parse().unwrap();
        assert!((0..3).all(|_| r.allow("s1", t0)));
        assert!(!r.allow("s1", t0));
        assert!(r.allow("s2", t0), "budgets are per key");
        assert!(r.allow("s1", t0 + SignedDuration::from_secs(20)), "one token refills every 20 s at 3/min");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-key-service --lib`
Expected: compile errors for the new modules.

- [ ] **Step 3: Implement**

`src/vault.rs`:

```rust
//! Sealed on start (spec §4.4). The root key exists only in memory, only after
//! `unseal` has matched the canary.

use std::sync::{Arc, RwLock};

use zeroize::Zeroizing;

use crate::root::RootKey;
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsealError { BadFormat, WrongKey, NotInitialised, Store }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError { AlreadyInitialised, Store }

pub struct Vault {
    store: Arc<Store>,
    root: RwLock<Option<Arc<RootKey>>>,
}

impl Vault {
    pub fn sealed(store: Arc<Store>) -> Self {
        Self { store, root: RwLock::new(None) }
    }

    pub fn is_sealed(&self) -> bool {
        self.root.read().expect("vault lock").is_none()
    }

    pub fn root(&self) -> Option<Arc<RootKey>> {
        self.root.read().expect("vault lock").clone()
    }

    pub async fn unseal(&self, encoded: &str) -> Result<(), UnsealError> {
        let key = RootKey::decode(encoded).map_err(|_| UnsealError::BadFormat)?;
        if !self.store.has_canary().await.map_err(|_| UnsealError::Store)? {
            return Err(UnsealError::NotInitialised);
        }
        if !self.store.check_canary(&key).await.map_err(|_| UnsealError::Store)? {
            return Err(UnsealError::WrongKey);
        }
        *self.root.write().expect("vault lock") = Some(Arc::new(key));
        Ok(())
    }
}

/// Generates the root key and writes the canary. The caller prints the returned
/// string once; it is never stored (spec §4.3).
pub async fn initialise(store: &Store) -> Result<Zeroizing<String>, InitError> {
    if store.has_canary().await.map_err(|_| InitError::Store)? {
        return Err(InitError::AlreadyInitialised);
    }
    let key = RootKey::generate().map_err(|_| InitError::Store)?;
    store.write_canary(&key).await.map_err(|_| InitError::Store)?;
    Ok(key.encode())
}
```

`src/epoch.rs`:

```rust
//! Chat epoch months (spec §3.2): Europe/Oslo calendar months, `YYYY-MM`.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Epoch { Past, Current, Future }

fn oslo() -> TimeZone {
    TimeZone::get("Europe/Oslo").expect("tzdb is bundled")
}

fn parse(month: &str) -> Option<Date> {
    let (y, m) = month.split_once('-')?;
    if y.len() != 4 || m.len() != 2 {
        return None;
    }
    Date::new(y.parse().ok()?, m.parse().ok()?, 1).ok()
}

pub fn current_month(now: Timestamp) -> String {
    let d = now.to_zoned(oslo()).date();
    format!("{:04}-{:02}", d.year(), d.month())
}

pub fn classify(month: &str, now: Timestamp) -> Option<Epoch> {
    let first = parse(month)?;
    let current = parse(&current_month(now))?;
    Some(match first.cmp(&current) {
        std::cmp::Ordering::Less => Epoch::Past,
        std::cmp::Ordering::Equal => Epoch::Current,
        std::cmp::Ordering::Greater => Epoch::Future,
    })
}

/// True once 12 months have passed since the month ended, in Oslo time.
pub fn may_destroy(month: &str, now: Timestamp) -> bool {
    let Some(first) = parse(month) else { return false };
    let Ok(expiry) = first.checked_add(13.months()) else { return false };
    let Ok(expiry) = expiry.to_zoned(oslo()) else { return false };
    now >= expiry.timestamp()
}
```

(A month ends when the next one starts, and 12 months after that is `first + 13 months`.)

`src/leases.rs`:

```rust
//! Leases make the concurrency ceiling enforceable (spec §3.3): the key service
//! counts what each backend instance still holds, and refuses a new distinct FAU
//! above the ceiling. The table holds no key material.

use std::collections::HashMap;
use std::sync::Mutex;

use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseError { CeilingReached, Unknown }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant { pub lease_id: Uuid, pub new_fau: bool }

struct Lease { instance: String, fau: Uuid, last_activity: Timestamp }

pub struct LeaseTable {
    idle: SignedDuration,
    ceiling: usize,
    leases: Mutex<HashMap<Uuid, Lease>>,
}

impl LeaseTable {
    pub fn new(idle: SignedDuration, ceiling: usize) -> Self {
        Self { idle, ceiling, leases: Mutex::new(HashMap::new()) }
    }

    pub fn idle(&self) -> SignedDuration {
        self.idle
    }

    fn sweep(&self, map: &mut HashMap<Uuid, Lease>, now: Timestamp) {
        map.retain(|_, l| l.last_activity + self.idle > now);
    }

    pub fn grant(&self, instance: &str, fau: Uuid, now: Timestamp) -> Result<Grant, LeaseError> {
        let mut map = self.leases.lock().expect("lease lock");
        self.sweep(&mut map, now);
        let mine: Vec<Uuid> = map.values().filter(|l| l.instance == instance).map(|l| l.fau).collect();
        let new_fau = !mine.contains(&fau);
        if new_fau {
            let mut distinct = mine;
            distinct.sort();
            distinct.dedup();
            if distinct.len() >= self.ceiling {
                return Err(LeaseError::CeilingReached);
            }
        }
        let lease_id = Uuid::now_v7();
        map.insert(lease_id, Lease { instance: instance.to_owned(), fau, last_activity: now });
        Ok(Grant { lease_id, new_fau })
    }

    pub fn renew(&self, lease: Uuid, instance: &str, now: Timestamp) -> Result<(), LeaseError> {
        let mut map = self.leases.lock().expect("lease lock");
        self.sweep(&mut map, now);
        match map.get_mut(&lease) {
            Some(l) if l.instance == instance => {
                l.last_activity = now;
                Ok(())
            }
            _ => Err(LeaseError::Unknown),
        }
    }

    /// Never counts as activity: a released lease is removed, not refreshed.
    pub fn release(&self, lease: Uuid, instance: &str) {
        let mut map = self.leases.lock().expect("lease lock");
        if map.get(&lease).is_some_and(|l| l.instance == instance) {
            map.remove(&lease);
        }
    }

    pub fn live(&self, now: Timestamp) -> usize {
        let mut map = self.leases.lock().expect("lease lock");
        self.sweep(&mut map, now);
        map.len()
    }
}
```

`src/limits.rs`:

```rust
//! Token buckets per key (a session id, or a backend instance), refilled continuously.

use std::collections::HashMap;
use std::sync::Mutex;

use jiff::Timestamp;

pub struct RateLimiter {
    per_minute: f64,
    buckets: Mutex<HashMap<String, (f64, Timestamp)>>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        Self { per_minute: f64::from(per_minute), buckets: Mutex::new(HashMap::new()) }
    }

    pub fn allow(&self, key: &str, now: Timestamp) -> bool {
        let mut map = self.buckets.lock().expect("limiter lock");
        let (tokens, at) = map.entry(key.to_owned()).or_insert((self.per_minute, now));
        let elapsed = now.duration_since(*at).as_secs_f64().max(0.0);
        *tokens = (*tokens + elapsed * self.per_minute / 60.0).min(self.per_minute);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}
```

`src/lib.rs`: add `pub mod epoch; pub mod leases; pub mod limits; pub mod vault;`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p fau-key-service --lib`
Expected: 9 (Task 3) + 2 + 3 + 4 + 1 = 19 passed.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/key-service
git commit -m "Add the key service's vault, chat-epoch rules, leases and rate limits (#3506)"
```

---

### Task 5: The API router, metrics and the audit log line

**Files:**
- Create: `backend/crates/key-service/src/{api,metrics}.rs`
- Modify: `src/lib.rs`
- Test: `backend/crates/key-service/tests/api.rs`, in-process through `tower::ServiceExt::oneshot`, with no TLS; TLS is Task 6

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  - `api::Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>`.
  - `api::PeerIdentity(pub String)`, which the TLS layer (Task 6) inserts as a request extension. It carries the client certificate's CN.
  - `api::ApiState::new(store: Arc<Store>, clock: Clock, limits: Limits) -> ApiState`. `ApiState` is `Clone` and exposes `.vault()` and `.metrics_text() -> String`.
  - `api::Limits { idle: SignedDuration, ceiling: usize, session_per_minute: u32, instance_per_minute: u32 }`, with `Limits::default()` giving 30 min, 200, 120 and 6000.
  - `api::router(state: ApiState) -> axum::Router`.
  - `api::metrics_router(state: ApiState) -> axum::Router` (`GET /metrics`).
  - `metrics::Metrics`, with counters and `render(sealed: bool, live_leases: usize) -> String`.

**The rules the handlers enforce** (spec §3.2):

| Route | Allowed classes | Notes |
|---|---|---|
| `POST /v1/keys/unwrap` | record, sealing (subject `None`); document, invitation (subject a UUID); chat_epoch (subject `YYYY-MM`, not a future month; the current month is created if missing) | `kek` gives 400 |
| `POST /v1/keys/create` | document, invitation (UUID subject); chat_epoch (current month only) | anything else gives 400; a non-current epoch gives 403 `refused` |
| `POST /v1/keys/destroy` | document; chat_epoch only when `may_destroy` | a young epoch gives 403 `refused`, increments `young_epoch_destroy_attempts` and logs at WARN |
| `POST /v1/faus/{id}/destroy` | n/a | 200 `{due_at}`, or 404; increments `fau_destructions_enqueued` and logs at WARN |
| `GET /v1/faus/{id}/public-key` | n/a | answers while sealed: the public key is not secret |
| `POST /v1/leases/*` | n/a | not gated by the seal; `release` never renews |

- Every key route except `public-key`, when the service is sealed, returns 503 `{"error":"sealed"}`.
- The order is: the seal check, then the rate limits (per session, then per instance), then the
  lease grant (ceiling), then the store. The lease is released again if the store has no such key.

| Error | Status |
|---|---|
| `Sealed` | 503 |
| `CeilingReached` | 429 |
| `RateLimited` | 429 |
| `NotFound` | 404 |
| `Refused` | 403 |
| `BadRequest` | 400 (including a malformed JSON body) |
| `Internal` | 500 |

- [ ] **Step 1: Write the failing tests** (`tests/api.rs`)

```rust
//! The key service's HTTP contract, in process. No TLS here; tests/tls.rs covers mTLS.

use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use fau_key_protocol::*;
use fau_key_service::api::{self, ApiState, Clock, Limits, PeerIdentity};
use fau_key_service::store::Store;
use fau_key_service::vault::initialise;
use jiff::{SignedDuration, Timestamp};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

struct Harness {
    _dir: tempfile::TempDir,
    state: ApiState,
    now: Arc<Mutex<Timestamp>>,
    root: String,
}

async fn harness(limits: Limits) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("k.db")).await.unwrap());
    let root = initialise(&store).await.unwrap().to_string();
    let now = Arc::new(Mutex::new("2026-09-27T12:00:00Z".parse::<Timestamp>().unwrap()));
    let n = now.clone();
    let clock: Clock = Arc::new(move || *n.lock().unwrap());
    let state = ApiState::new(store, clock, limits);
    Harness { _dir: dir, state, now, root }
}

impl Harness {
    async fn unseal(&self) {
        self.state.vault().unseal(&self.root).await.unwrap();
    }
    fn advance(&self, d: SignedDuration) {
        let mut n = self.now.lock().unwrap();
        *n = *n + d;
    }
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder().method(method).uri(path).header("content-type", "application/json")
            .extension(PeerIdentity("fau-app".into()))
            .body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
        let res = api::router(self.state.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
}

fn caller() -> Value {
    json!({ "account_id": Uuid::now_v7(), "session_id": Uuid::now_v7(), "instance_id": "app-0" })
}

async fn new_fau(h: &Harness) -> Uuid {
    let fau = Uuid::now_v7();
    let (s, _) = h.call("POST", "/v1/faus", Some(json!({ "fau_id": fau, "caller": caller() }))).await;
    assert_eq!(s, StatusCode::OK);
    fau
}

#[tokio::test]
async fn sealed_answers_only_sealed_and_ready_says_so() {
    let h = harness(Limits::default()).await;
    let (s, b) = h.call("POST", "/v1/keys/unwrap", Some(json!({ "fau_id": Uuid::now_v7(), "class": "record", "subject": null, "caller": caller() }))).await;
    assert_eq!((s, b), (StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "sealed" })));
    let (s, b) = h.call("POST", "/v1/faus", Some(json!({ "fau_id": Uuid::now_v7(), "caller": caller() }))).await;
    assert_eq!((s, b["error"].clone()), (StatusCode::SERVICE_UNAVAILABLE, json!("sealed")));
    let (s, b) = h.call("GET", "/health/ready", None).await;
    assert_eq!((s, b), (StatusCode::OK, json!({ "sealed": true })));
    assert!(h.state.metrics_text().contains("key_service_sealed 1"));
}

#[tokio::test]
async fn the_record_key_is_the_same_key_every_time_under_a_fresh_lease() {
    let h = harness(Limits::default()).await;
    h.unseal().await;
    let fau = new_fau(&h).await;
    let req = json!({ "fau_id": fau, "class": "record", "subject": null, "caller": caller() });
    let (s1, a) = h.call("POST", "/v1/keys/unwrap", Some(req.clone())).await;
    let (s2, b) = h.call("POST", "/v1/keys/unwrap", Some(req)).await;
    assert_eq!((s1, s2), (StatusCode::OK, StatusCode::OK));
    assert_eq!(a["key"], b["key"]);
    assert_ne!(a["lease_id"], b["lease_id"]);
    assert_eq!(a["idle_timeout_secs"], json!(1800));
    let (s, pk) = h.call("GET", &format!("/v1/faus/{fau}/public-key"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(pk["public_key"].as_str().unwrap().len() == 43);
}

#[tokio::test]
async fn the_kek_and_unknown_subjects_are_never_served() {
    let h = harness(Limits::default()).await;
    h.unseal().await;
    let fau = new_fau(&h).await;
    let (s, _) = h.call("POST", "/v1/keys/unwrap", Some(json!({ "fau_id": fau, "class": "kek", "subject": null, "caller": caller() }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = h.call("POST", "/v1/keys/unwrap", Some(json!({ "fau_id": fau, "class": "document", "subject": Uuid::now_v7(), "caller": caller() }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = h.call("POST", "/v1/keys/unwrap", Some(json!({ "fau_id": fau, "class": "document", "subject": "not-a-uuid", "caller": caller() }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = h.call("POST", "/v1/keys/unwrap", Some(json!({ "fau_id": Uuid::now_v7(), "class": "record", "subject": null, "caller": caller() }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "an unknown FAU");
}

#[tokio::test]
async fn chat_epochs_follow_the_oslo_calendar_and_the_12_month_rule() {
    let h = harness(Limits::default()).await;
    h.unseal().await;
    let fau = new_fau(&h).await;
    let unwrap = |m: &str| json!({ "fau_id": fau, "class": "chat_epoch", "subject": m, "caller": caller() });
    let (s, current) = h.call("POST", "/v1/keys/unwrap", Some(unwrap("2026-09"))).await;
    assert_eq!(s, StatusCode::OK, "the current month is created lazily");
    let (s, _) = h.call("POST", "/v1/keys/unwrap", Some(unwrap("2026-10"))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a future month is refused");
    let (s, _) = h.call("POST", "/v1/keys/create", Some(unwrap("2026-08"))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a past month cannot be created");
    let destroy = json!({ "fau_id": fau, "class": "chat_epoch", "subject": "2026-09", "caller": caller() });
    let (s, _) = h.call("POST", "/v1/keys/destroy", Some(destroy.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(h.state.metrics_text().contains("key_service_young_epoch_destroy_attempts_total 1"));
    h.advance(SignedDuration::from_hours(24 * 400));
    let (s, again) = h.call("POST", "/v1/keys/unwrap", Some(unwrap("2026-09"))).await;
    assert_eq!((s, &again["key"]), (StatusCode::OK, &current["key"]), "a past month still unwraps");
    let (s, _) = h.call("POST", "/v1/keys/destroy", Some(destroy)).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = h.call("POST", "/v1/keys/unwrap", Some(unwrap("2026-09"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_ceiling_refuses_a_new_fau_and_counts_it() {
    let h = harness(Limits { ceiling: 1, ..Limits::default() }).await;
    h.unseal().await;
    let (a, b) = (new_fau(&h).await, new_fau(&h).await);
    let rec = |f: Uuid| json!({ "fau_id": f, "class": "record", "subject": null, "caller": caller() });
    assert_eq!(h.call("POST", "/v1/keys/unwrap", Some(rec(a))).await.0, StatusCode::OK);
    let (s, body) = h.call("POST", "/v1/keys/unwrap", Some(rec(b))).await;
    assert_eq!((s, body), (StatusCode::TOO_MANY_REQUESTS, json!({ "error": "ceiling_reached" })));
    assert!(h.state.metrics_text().contains("key_service_ceiling_refusals_total 1"));
    assert!(h.state.metrics_text().contains("key_service_new_fau_leases_total 1"));
}

#[tokio::test]
async fn a_session_over_its_budget_is_rate_limited() {
    let h = harness(Limits { session_per_minute: 2, ..Limits::default() }).await;
    h.unseal().await;
    let fau = new_fau(&h).await;
    let c = caller();
    let req = json!({ "fau_id": fau, "class": "record", "subject": null, "caller": c });
    assert_eq!(h.call("POST", "/v1/keys/unwrap", Some(req.clone())).await.0, StatusCode::OK);
    assert_eq!(h.call("POST", "/v1/keys/unwrap", Some(req.clone())).await.0, StatusCode::OK);
    assert_eq!(h.call("POST", "/v1/keys/unwrap", Some(req)).await.1, json!({ "error": "rate_limited" }));
}

#[tokio::test]
async fn destroying_an_fau_queues_it_and_stops_its_keys() {
    let h = harness(Limits::default()).await;
    h.unseal().await;
    let fau = new_fau(&h).await;
    let (s, b) = h.call("POST", &format!("/v1/faus/{fau}/destroy"), Some(json!({ "caller": caller() }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["due_at"], json!("2026-10-04T12:00:00Z"));
    let (s, _) = h.call("POST", "/v1/keys/unwrap", Some(json!({ "fau_id": fau, "class": "record", "subject": null, "caller": caller() }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(h.state.metrics_text().contains("key_service_fau_destructions_enqueued_total 1"));
}

#[tokio::test]
async fn leases_renew_and_release_but_release_is_not_renewal() {
    let h = harness(Limits::default()).await;
    h.unseal().await;
    let fau = new_fau(&h).await;
    let (_, k) = h.call("POST", "/v1/keys/unwrap", Some(json!({ "fau_id": fau, "class": "record", "subject": null, "caller": caller() }))).await;
    let lease = json!({ "lease_id": k["lease_id"], "instance_id": "app-0" });
    assert_eq!(h.call("POST", "/v1/leases/renew", Some(lease.clone())).await.0, StatusCode::OK);
    assert_eq!(h.call("POST", "/v1/leases/release", Some(lease.clone())).await.0, StatusCode::OK);
    assert_eq!(h.call("POST", "/v1/leases/renew", Some(lease)).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_router_serves_exactly_the_pinned_routes() {
    let h = harness(Limits::default()).await;
    for (method, path) in ROUTES {
        let concrete = path.replace("{fau_id}", &Uuid::now_v7().to_string());
        let (s, body) = h.call(method, &concrete, if *method == "POST" { Some(json!({})) } else { None }).await;
        // A routed 404 carries {"error":"not_found"}; axum's unrouted 404 has no body.
        assert!(!(s == StatusCode::NOT_FOUND && body.is_null()), "{method} {path} is not routed");
        assert_ne!(s, StatusCode::METHOD_NOT_ALLOWED, "{method} {path} is not routed");
    }
    let (s, body) = h.call("GET", "/v1/keys", None).await;
    assert!(s == StatusCode::NOT_FOUND && body.is_null());
    // Blunt on purpose: a new `.route(` in api.rs must come with a new ROUTES entry.
    let source = include_str!("../src/api.rs");
    assert_eq!(source.matches(".route(\"").count(), ROUTES.len());
}

#[tokio::test]
async fn no_key_material_reaches_logs_or_metrics() {
    use std::io::Write;
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.0.lock().unwrap().extend_from_slice(b); Ok(b.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let buf = Buf::default();
    let b2 = buf.clone();
    let subscriber = tracing_subscriber::fmt().json().with_writer(move || b2.clone()).finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = harness(Limits::default()).await;
    h.unseal().await;
    let fau = new_fau(&h).await;
    let mut keys = Vec::new();
    for (class, subject) in [("record", json!(null)), ("sealing", json!(null)), ("chat_epoch", json!("2026-09"))] {
        let (_, k) = h.call("POST", "/v1/keys/unwrap", Some(json!({ "fau_id": fau, "class": class, "subject": subject, "caller": caller() }))).await;
        keys.push(k["key"].as_str().unwrap().to_owned());
    }
    let logs = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("\"op\":\"unwrap\""), "the audit line is written");
    let metrics = h.state.metrics_text();
    for secret in keys.iter().chain(std::iter::once(&h.root)) {
        assert!(!logs.contains(secret.as_str()), "key material in a log line");
        assert!(!metrics.contains(secret.as_str()), "key material in a metric");
    }
}
```

Add to `[dev-dependencies]` in the key-service `Cargo.toml`:
`fau-key-protocol = { path = "../key-protocol" }` (already a dependency) and
`tracing-subscriber = { workspace = true }` (already a dependency). No change is needed if both
are present.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-key-service --test api`
Expected: compile errors (`api` module missing).

- [ ] **Step 3: Implement `src/metrics.rs`**

```rust
//! Hand-rendered Prometheus text: nine series, no dependency. Values are counts,
//! never identifiers, so no series can carry key material.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub unseal_failures: AtomicU64,
    pub unwraps: AtomicU64,
    pub new_fau_leases: AtomicU64,
    pub ceiling_refusals: AtomicU64,
    pub rate_limited: AtomicU64,
    pub fau_destructions_enqueued: AtomicU64,
    pub young_epoch_destroy_attempts: AtomicU64,
}

pub fn inc(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

impl Metrics {
    pub fn render(&self, sealed: bool, live_leases: usize) -> String {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let mut out = String::new();
        let mut line = |name: &str, kind: &str, v: u64| {
            out.push_str(&format!("# TYPE {name} {kind}\n{name} {v}\n"));
        };
        line("key_service_sealed", "gauge", u64::from(sealed));
        line("key_service_live_leases", "gauge", live_leases as u64);
        line("key_service_unseal_failures_total", "counter", g(&self.unseal_failures));
        line("key_service_unwraps_total", "counter", g(&self.unwraps));
        line("key_service_new_fau_leases_total", "counter", g(&self.new_fau_leases));
        line("key_service_ceiling_refusals_total", "counter", g(&self.ceiling_refusals));
        line("key_service_rate_limited_total", "counter", g(&self.rate_limited));
        line("key_service_fau_destructions_enqueued_total", "counter", g(&self.fau_destructions_enqueued));
        line("key_service_young_epoch_destroy_attempts_total", "counter", g(&self.young_epoch_destroy_attempts));
        out
    }
}
```

- [ ] **Step 4: Implement `src/api.rs`**

```rust
//! The key service's API (spec §3.2). Exactly the routes in `fau_key_protocol::ROUTES`.
//! One key per call; every served key is under a lease; every call writes one audit line.

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use fau_key_protocol::*;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::epoch::{self, Epoch};
use crate::leases::{LeaseError, LeaseTable};
use crate::limits::RateLimiter;
use crate::metrics::{inc, Metrics};
use crate::root::RootKey;
use crate::store::{Store, StoreError};
use crate::vault::Vault;

pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The verified client certificate's common name, inserted by the TLS layer.
#[derive(Debug, Clone)]
pub struct PeerIdentity(pub String);

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub idle: SignedDuration,
    pub ceiling: usize,
    pub session_per_minute: u32,
    pub instance_per_minute: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { idle: SignedDuration::from_mins(30), ceiling: 200, session_per_minute: 120, instance_per_minute: 6000 }
    }
}

struct Inner {
    vault: Vault,
    store: Arc<Store>,
    leases: LeaseTable,
    session_limit: RateLimiter,
    instance_limit: RateLimiter,
    metrics: Metrics,
    clock: Clock,
}

#[derive(Clone)]
pub struct ApiState(Arc<Inner>);

impl ApiState {
    pub fn new(store: Arc<Store>, clock: Clock, limits: Limits) -> Self {
        Self(Arc::new(Inner {
            vault: Vault::sealed(store.clone()),
            store,
            leases: LeaseTable::new(limits.idle, limits.ceiling),
            session_limit: RateLimiter::new(limits.session_per_minute),
            instance_limit: RateLimiter::new(limits.instance_per_minute),
            metrics: Metrics::default(),
            clock,
        }))
    }

    pub fn vault(&self) -> &Vault {
        &self.0.vault
    }

    pub fn metrics(&self) -> &Metrics {
        &self.0.metrics
    }

    pub fn metrics_text(&self) -> String {
        self.0.metrics.render(self.0.vault.is_sealed(), self.0.leases.live((self.0.clock)()))
    }
}

pub struct Fail(ErrorCode);

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ErrorCode::Sealed => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::CeilingReached | ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Refused => StatusCode::FORBIDDEN,
            ErrorCode::BadRequest => StatusCode::BAD_REQUEST,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(ErrorBody { error: self.0 })).into_response()
    }
}

impl From<StoreError> for Fail {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::FauUnknown => Fail(ErrorCode::NotFound),
            StoreError::Sqlite(_) | StoreError::Corrupt => {
                tracing::error!(error = %e, "key store failure");
                Fail(ErrorCode::Internal)
            }
        }
    }
}

impl From<JsonRejection> for Fail {
    fn from(_: JsonRejection) -> Self {
        Fail(ErrorCode::BadRequest)
    }
}

/// One JSON line per call (spec §4.2). Identifiers only: never a key, nonce or ciphertext.
#[allow(clippy::too_many_arguments)]
fn audit(op: &str, class: Option<KeyClass>, fau: Option<Uuid>, subject: Option<&str>, caller: Option<&Caller>, peer: &str, lease: Option<Uuid>, outcome: &str) {
    tracing::info!(
        op, outcome, peer,
        class = class.map(KeyClass::code),
        fau_id = fau.map(|f| f.to_string()),
        subject,
        account_id = caller.map(|c| c.account_id.to_string()),
        session_id = caller.map(|c| c.session_id.to_string()),
        instance_id = caller.map(|c| c.instance_id.as_str()),
        lease_id = lease.map(|l| l.to_string()),
        "key service call"
    );
}

fn outcome(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Sealed => "sealed",
        ErrorCode::CeilingReached => "ceiling_reached",
        ErrorCode::RateLimited => "rate_limited",
        ErrorCode::NotFound => "not_found",
        ErrorCode::Refused => "refused",
        ErrorCode::BadRequest => "bad_request",
        ErrorCode::Internal => "internal",
    }
}

impl ApiState {
    fn now(&self) -> Timestamp {
        (self.0.clock)()
    }

    fn instance(peer: &PeerIdentity, instance_id: &str) -> String {
        format!("{}|{}", peer.0, instance_id)
    }

    /// Seal first, then the session budget, then the instance budget.
    fn admit(&self, peer: &PeerIdentity, caller: &Caller, now: Timestamp) -> Result<Arc<RootKey>, Fail> {
        let root = self.0.vault.root().ok_or(Fail(ErrorCode::Sealed))?;
        if !self.0.session_limit.allow(&caller.session_id.to_string(), now)
            || !self.0.instance_limit.allow(&Self::instance(peer, &caller.instance_id), now)
        {
            inc(&self.0.metrics.rate_limited);
            return Err(Fail(ErrorCode::RateLimited));
        }
        Ok(root)
    }

    fn lease(&self, peer: &PeerIdentity, caller: &Caller, fau: Uuid, now: Timestamp) -> Result<Uuid, Fail> {
        match self.0.leases.grant(&Self::instance(peer, &caller.instance_id), fau, now) {
            Ok(g) => {
                if g.new_fau {
                    inc(&self.0.metrics.new_fau_leases);
                }
                Ok(g.lease_id)
            }
            Err(LeaseError::CeilingReached) => {
                inc(&self.0.metrics.ceiling_refusals);
                tracing::warn!(peer = %peer.0, instance_id = %caller.instance_id, "key service ceiling reached");
                Err(Fail(ErrorCode::CeilingReached))
            }
            Err(LeaseError::Unknown) => Err(Fail(ErrorCode::Internal)),
        }
    }

    fn key_response(&self, key_id: Uuid, key: &Zeroizing<[u8; 32]>, lease_id: Uuid) -> KeyResponse {
        inc(&self.0.metrics.unwraps);
        KeyResponse { key_id, key: encode_key(key), lease_id, idle_timeout_secs: self.0.leases.idle().as_secs() as u64 }
    }
}

enum Purpose { Unwrap, Create, Destroy }

/// Validates the subject for the class and purpose (spec §3.2). Returns the subject to
/// store and, for chat epochs, the epoch's position relative to now.
fn subject_for(class: KeyClass, subject: Option<&str>, purpose: Purpose, now: Timestamp) -> Result<Option<String>, Fail> {
    let bad = || Fail(ErrorCode::BadRequest);
    match (class, purpose) {
        (KeyClass::Kek, _) => Err(bad()),
        (KeyClass::Record | KeyClass::Sealing, Purpose::Unwrap) => subject.is_none().then_some(None).ok_or_else(bad),
        (KeyClass::Record | KeyClass::Sealing, _) => Err(bad()),
        (KeyClass::Invitation, Purpose::Destroy) => Err(bad()),
        (KeyClass::Document | KeyClass::Invitation, _) => {
            let s = subject.ok_or_else(bad)?;
            Uuid::parse_str(s).map_err(|_| bad())?;
            Ok(Some(s.to_owned()))
        }
        (KeyClass::ChatEpoch, p) => {
            let s = subject.ok_or_else(bad)?;
            let e = epoch::classify(s, now).ok_or_else(bad)?;
            match (p, e) {
                (Purpose::Unwrap, Epoch::Future) | (Purpose::Create, Epoch::Past | Epoch::Future) => Err(Fail(ErrorCode::Refused)),
                _ => Ok(Some(s.to_owned())),
            }
        }
    }
}

async fn create_fau(State(s): State<ApiState>, Extension(peer): Extension<PeerIdentity>, body: Result<Json<CreateFauRequest>, JsonRejection>) -> Result<Json<PublicKeyResponse>, Fail> {
    let Json(req) = body?;
    let now = s.now();
    let root = s.admit(&peer, &req.caller, now).inspect_err(|f| audit("create_fau", None, Some(req.fau_id), None, Some(&req.caller), &peer.0, None, outcome(f.0)))?;
    let pk = s.0.store.create_fau(&root, req.fau_id).await?;
    audit("create_fau", None, Some(req.fau_id), None, Some(&req.caller), &peer.0, None, "ok");
    Ok(Json(PublicKeyResponse { public_key: encode_key(&pk) }))
}

async fn public_key(State(s): State<ApiState>, Path(fau): Path<Uuid>) -> Result<Json<PublicKeyResponse>, Fail> {
    let pk = s.0.store.public_key(fau).await?.ok_or(Fail(ErrorCode::NotFound))?;
    Ok(Json(PublicKeyResponse { public_key: encode_key(&pk) }))
}

async fn unwrap(State(s): State<ApiState>, Extension(peer): Extension<PeerIdentity>, body: Result<Json<UnwrapRequest>, JsonRejection>) -> Result<Json<KeyResponse>, Fail> {
    let Json(req) = body?;
    let now = s.now();
    let log = |lease: Option<Uuid>, o: &str| audit("unwrap", Some(req.class), Some(req.fau_id), req.subject.as_deref(), Some(&req.caller), &peer.0, lease, o);
    let result = async {
        let root = s.admit(&peer, &req.caller, now)?;
        let subject = subject_for(req.class, req.subject.as_deref(), Purpose::Unwrap, now)?;
        let lease = s.lease(&peer, &req.caller, req.fau_id, now)?;
        let found = match (req.class, subject.as_deref()) {
            (KeyClass::ChatEpoch, Some(m)) if epoch::classify(m, now) == Some(Epoch::Current) => {
                Some(s.0.store.create_key(&root, req.fau_id, req.class, m).await)
            }
            _ => s.0.store.unwrap_key(&root, req.fau_id, req.class, subject.as_deref()).await.transpose(),
        };
        match found {
            Some(Ok((id, key))) => Ok(s.key_response(id, &key, lease)),
            Some(Err(e)) => {
                s.0.leases.release(lease, &ApiState::instance(&peer, &req.caller.instance_id));
                Err(Fail::from(e))
            }
            None => {
                s.0.leases.release(lease, &ApiState::instance(&peer, &req.caller.instance_id));
                Err(Fail(ErrorCode::NotFound))
            }
        }
    }
    .await;
    match &result {
        Ok(r) => log(Some(r.lease_id), "ok"),
        Err(f) => log(None, outcome(f.0)),
    }
    result.map(Json)
}

async fn create(State(s): State<ApiState>, Extension(peer): Extension<PeerIdentity>, body: Result<Json<CreateKeyRequest>, JsonRejection>) -> Result<Json<KeyResponse>, Fail> {
    let Json(req) = body?;
    let now = s.now();
    let result = async {
        let root = s.admit(&peer, &req.caller, now)?;
        let subject = subject_for(req.class, Some(&req.subject), Purpose::Create, now)?.ok_or(Fail(ErrorCode::BadRequest))?;
        let lease = s.lease(&peer, &req.caller, req.fau_id, now)?;
        match s.0.store.create_key(&root, req.fau_id, req.class, &subject).await {
            Ok((id, key)) => Ok(s.key_response(id, &key, lease)),
            Err(e) => {
                s.0.leases.release(lease, &ApiState::instance(&peer, &req.caller.instance_id));
                Err(Fail::from(e))
            }
        }
    }
    .await;
    audit("create", Some(req.class), Some(req.fau_id), Some(&req.subject), Some(&req.caller), &peer.0,
        result.as_ref().ok().map(|r| r.lease_id), result.as_ref().map_or_else(|f| outcome(f.0), |_| "ok"));
    result.map(Json)
}

async fn destroy(State(s): State<ApiState>, Extension(peer): Extension<PeerIdentity>, body: Result<Json<DestroyKeyRequest>, JsonRejection>) -> Result<Json<serde_json::Value>, Fail> {
    let Json(req) = body?;
    let now = s.now();
    let result = async {
        s.admit(&peer, &req.caller, now)?;
        let subject = subject_for(req.class, Some(&req.subject), Purpose::Destroy, now)?.ok_or(Fail(ErrorCode::BadRequest))?;
        if req.class == KeyClass::ChatEpoch && !epoch::may_destroy(&subject, now) {
            inc(&s.0.metrics.young_epoch_destroy_attempts);
            tracing::warn!(fau_id = %req.fau_id, subject = %subject, "refused to destroy a chat epoch younger than 12 months");
            return Err(Fail(ErrorCode::Refused));
        }
        if s.0.store.destroy_key(req.fau_id, req.class, &subject).await? { Ok(()) } else { Err(Fail(ErrorCode::NotFound)) }
    }
    .await;
    audit("destroy", Some(req.class), Some(req.fau_id), Some(&req.subject), Some(&req.caller), &peer.0, None,
        result.as_ref().map_or_else(|f| outcome(f.0), |_| "ok"));
    result.map(|_| Json(json!({})))
}

async fn destroy_fau(State(s): State<ApiState>, Extension(peer): Extension<PeerIdentity>, Path(fau): Path<Uuid>, body: Result<Json<DestroyFauRequest>, JsonRejection>) -> Result<Json<DestroyFauResponse>, Fail> {
    let Json(req) = body?;
    let now = s.now();
    let result = async {
        s.admit(&peer, &req.caller, now)?;
        let due = s.0.store.destroy_fau(fau, now).await?.ok_or(Fail(ErrorCode::NotFound))?;
        inc(&s.0.metrics.fau_destructions_enqueued);
        tracing::warn!(fau_id = %fau, due_at = %due, "FAU key destruction enqueued");
        Ok(due)
    }
    .await;
    audit("destroy_fau", Some(KeyClass::Kek), Some(fau), None, Some(&req.caller), &peer.0, None,
        result.as_ref().map_or_else(|f: &Fail| outcome(f.0), |_| "ok"));
    result.map(|due| Json(DestroyFauResponse { due_at: due.to_string() }))
}

async fn renew(State(s): State<ApiState>, Extension(peer): Extension<PeerIdentity>, body: Result<Json<LeaseRequest>, JsonRejection>) -> Result<Json<LeaseResponse>, Fail> {
    let Json(req) = body?;
    s.0.leases.renew(req.lease_id, &ApiState::instance(&peer, &req.instance_id), s.now()).map_err(|_| Fail(ErrorCode::NotFound))?;
    Ok(Json(LeaseResponse { idle_timeout_secs: s.0.leases.idle().as_secs() as u64 }))
}

async fn release(State(s): State<ApiState>, Extension(peer): Extension<PeerIdentity>, body: Result<Json<LeaseRequest>, JsonRejection>) -> Result<Json<serde_json::Value>, Fail> {
    let Json(req) = body?;
    s.0.leases.release(req.lease_id, &ApiState::instance(&peer, &req.instance_id));
    Ok(Json(json!({})))
}

async fn live() -> &'static str {
    "ok"
}

async fn ready(State(s): State<ApiState>) -> Json<serde_json::Value> {
    Json(json!({ "sealed": s.0.vault.is_sealed() }))
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/v1/faus", post(create_fau))
        .route("/v1/faus/{fau_id}/public-key", get(public_key))
        .route("/v1/faus/{fau_id}/destroy", post(destroy_fau))
        .route("/v1/keys/create", post(create))
        .route("/v1/keys/unwrap", post(unwrap))
        .route("/v1/keys/destroy", post(destroy))
        .route("/v1/leases/renew", post(renew))
        .route("/v1/leases/release", post(release))
        .with_state(state)
}

async fn metrics(State(s): State<ApiState>) -> String {
    s.metrics_text()
}

/// Served on its own listener (spec §6.2), reachable from monitoring only. Written
/// with `concat!` so the route-count test in tests/api.rs counts API routes only.
pub fn metrics_router(state: ApiState) -> Router {
    Router::new().route(concat!("/", "metrics"), get(metrics)).with_state(state)
}
```

`src/lib.rs`: add `pub mod api; pub mod metrics;`.

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test -p fau-key-service`
Expected: 19 lib + 10 api = 29 passed. Common fixes:
- `inspect_err` on `Result` needs Rust ≥ 1.76 (fine on 1.98).
- A `Json` rejection from an empty `{}` body on the route-pin test must give 400, not 422. The
  `Result<Json<_>, JsonRejection>` extractor does this.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/key-service
git commit -m "Add the key service API, metrics and audit log line (#3506)"
```

---

### Task 6: mTLS, dev certificates and the in-process test server

**Files:**
- Create: `backend/crates/key-service/src/{tls,devcerts,test_support}.rs`
- Modify: `src/lib.rs`, `Cargo.toml` (`tempfile` becomes an optional dependency for `test-support`)
- Test: `backend/crates/key-service/tests/tls.rs`

**Interfaces:**
- Consumes: `api::router`, `api::PeerIdentity`, `api::ApiState`, `api::Limits` (Task 5).
- Produces:
  - `tls::TlsMaterial { cert_pem: String, key_pem: String, client_ca_pem: String }`.
  - `tls::server_config(&TlsMaterial) -> Result<Arc<rustls::ServerConfig>, TlsError>`.
  - `tls::serve_mtls(listener: TcpListener, config: Arc<ServerConfig>, app: Router, shutdown: impl Future<Output = ()> + Send + 'static)`, which returns when shutdown fires.
  - `devcerts::DevCerts { ca_pem, server_cert_pem, server_key_pem, client_cert_pem, client_key_pem }`.
  - `devcerts::generate(server_names: &[String], client_cn: &str) -> DevCerts`.
  - `devcerts::issue_client(&DevCerts, cn: &str) -> (String /*cert*/, String /*key*/)`.
  - `DevCerts::write_to(dir: &Path) -> io::Result<()>`, writing `ca.pem`, `server.pem`, `server-key.pem`, `client.pem` and `client-key.pem`. Key files get mode 0600.
  - Behind `feature = "test-support"`: `test_support::TestServer { base_url: String, ca_pem: String, client_identity_pem: String, root_key: String, state: ApiState }` and `test_support::spawn(limits: Limits, unsealed: bool) -> TestServer`. `TestServer::identity_for(cn: &str) -> String` issues another client's identity PEM from the same CA.

- [ ] **Step 1: Write the failing tests** (`tests/tls.rs`)

```rust
//! mTLS (spec §5.2): the key service answers only clients whose certificate chains to
//! its client CA, and the ceiling is per certificate identity.

#![cfg(feature = "test-support")]

use fau_key_service::api::Limits;
use fau_key_service::test_support::spawn;
use serde_json::{json, Value};
use uuid::Uuid;

fn client(ca_pem: &str, identity: Option<&str>) -> reqwest::Client {
    let mut b = reqwest::Client::builder().tls_certs_only([reqwest::Certificate::from_pem(ca_pem.as_bytes()).unwrap()]);
    if let Some(id) = identity {
        b = b.identity(reqwest::Identity::from_pem(id.as_bytes()).unwrap());
    }
    b.build().unwrap()
}

#[tokio::test]
async fn a_client_with_the_right_certificate_is_served() {
    let srv = spawn(Limits::default(), true).await;
    let c = client(&srv.ca_pem, Some(&srv.client_identity_pem));
    let v: Value = c.get(format!("{}/health/ready", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(v, json!({ "sealed": false }));
}

#[tokio::test]
async fn a_client_without_a_certificate_is_refused_at_the_handshake() {
    let srv = spawn(Limits::default(), true).await;
    assert!(client(&srv.ca_pem, None).get(format!("{}/health/live", srv.base_url)).send().await.is_err());
}

#[tokio::test]
async fn a_certificate_from_another_ca_is_refused() {
    let srv = spawn(Limits::default(), true).await;
    let stranger = fau_key_service::devcerts::generate(&["localhost".into()], "fau-app");
    let identity = format!("{}{}", stranger.client_cert_pem, stranger.client_key_pem);
    assert!(client(&srv.ca_pem, Some(&identity)).get(format!("{}/health/live", srv.base_url)).send().await.is_err());
}

#[tokio::test]
async fn the_ceiling_is_per_certificate_identity() {
    let srv = spawn(Limits { ceiling: 1, ..Limits::default() }, true).await;
    let a = client(&srv.ca_pem, Some(&srv.client_identity_pem));
    let b = client(&srv.ca_pem, Some(&srv.identity_for("fau-app-other")));
    let caller = json!({ "account_id": Uuid::now_v7(), "session_id": Uuid::now_v7(), "instance_id": "same-pod-name" });
    let mut faus = Vec::new();
    for _ in 0..2 {
        let f = Uuid::now_v7();
        assert!(a.post(format!("{}/v1/faus", srv.base_url)).json(&json!({ "fau_id": f, "caller": caller })).send().await.unwrap().status().is_success());
        faus.push(f);
    }
    let unwrap = |f: Uuid| json!({ "fau_id": f, "class": "record", "subject": null, "caller": caller });
    assert_eq!(a.post(format!("{}/v1/keys/unwrap", srv.base_url)).json(&unwrap(faus[0])).send().await.unwrap().status(), 200);
    assert_eq!(a.post(format!("{}/v1/keys/unwrap", srv.base_url)).json(&unwrap(faus[1])).send().await.unwrap().status(), 429);
    assert_eq!(b.post(format!("{}/v1/keys/unwrap", srv.base_url)).json(&unwrap(faus[1])).send().await.unwrap().status(), 200);
}
```

Add `reqwest` to `[dev-dependencies]` as it already is in `[dependencies]`; no change is needed.
Run these with the feature enabled (Step 4).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-key-service --features test-support --test tls`
Expected: compile errors (`test_support`, `devcerts` missing).

- [ ] **Step 3: Implement**

In `Cargo.toml`, change the feature to `test-support = ["dep:tempfile"]` and add
`tempfile = { version = "3", optional = true }` under `[dependencies]`. Keep it in
`[dev-dependencies]` too.

`src/devcerts.rs`:

```rust
//! A throwaway CA with one server and one client certificate, for compose and tests
//! (spec §5.2). Production certificates come from cert-manager's internal CA Issuer.

use std::io;
use std::path::Path;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};

pub struct DevCerts {
    pub ca_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
    ca_params: CertificateParams,
    ca_key_pem: String,
}

fn ca() -> (CertificateParams, KeyPair, String) {
    let key = KeyPair::generate().expect("keypair");
    let mut p = CertificateParams::new(Vec::<String>::new()).expect("params");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.distinguished_name.push(DnType::CommonName, "fau-keys-dev-ca");
    let pem = p.self_signed(&key).expect("self-signed").pem();
    (p, key, pem)
}

fn issue(ca_params: &CertificateParams, ca_key: KeyPair, sans: Vec<String>, cn: &str) -> (String, String) {
    let issuer = Issuer::new(ca_params.clone(), ca_key);
    let key = KeyPair::generate().expect("keypair");
    let mut p = CertificateParams::new(sans).expect("params");
    p.distinguished_name.push(DnType::CommonName, cn);
    (p.signed_by(&key, &issuer).expect("sign").pem(), key.serialize_pem())
}

pub fn generate(server_names: &[String], client_cn: &str) -> DevCerts {
    let (ca_params, ca_key, ca_pem) = ca();
    let ca_key_pem = ca_key.serialize_pem();
    let (server_cert_pem, server_key_pem) = issue(&ca_params, KeyPair::from_pem(&ca_key_pem).expect("ca key"), server_names.to_vec(), "fau-keys");
    let (client_cert_pem, client_key_pem) = issue(&ca_params, KeyPair::from_pem(&ca_key_pem).expect("ca key"), vec![], client_cn);
    DevCerts { ca_pem, server_cert_pem, server_key_pem, client_cert_pem, client_key_pem, ca_params, ca_key_pem }
}

pub fn issue_client(certs: &DevCerts, cn: &str) -> (String, String) {
    issue(&certs.ca_params, KeyPair::from_pem(&certs.ca_key_pem).expect("ca key"), vec![], cn)
}

impl DevCerts {
    pub fn write_to(&self, dir: &Path) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir)?;
        for (name, body, secret) in [
            ("ca.pem", &self.ca_pem, false),
            ("server.pem", &self.server_cert_pem, false),
            ("server-key.pem", &self.server_key_pem, true),
            ("client.pem", &self.client_cert_pem, false),
            ("client-key.pem", &self.client_key_pem, true),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, body)?;
            if secret {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
        }
        Ok(())
    }
}
```

The CA's private key is never written to disk. `dev-certs` produces a CA that can issue nothing
further once the process exits, which is fine for development.

`src/tls.rs`:

```rust
//! The mTLS accept loop (spec §5.2). Verifies the client certificate against the
//! client CA and inserts its CN as `PeerIdentity` on every request.

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tower::Service;

use crate::api::PeerIdentity;

pub struct TlsMaterial {
    pub cert_pem: String,
    pub key_pem: String,
    pub client_ca_pem: String,
}

#[derive(Debug, thiserror::Error)]
#[error("the TLS material could not be loaded: {0}")]
pub struct TlsError(&'static str);

pub fn server_config(m: &TlsMaterial) -> Result<Arc<ServerConfig>, TlsError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(m.cert_pem.as_bytes())
        .collect::<Result<_, _>>().map_err(|_| TlsError("server certificate"))?;
    let key = PrivateKeyDer::from_pem_slice(m.key_pem.as_bytes()).map_err(|_| TlsError("server key"))?;
    let mut roots = RootCertStore::empty();
    for ca in CertificateDer::pem_slice_iter(m.client_ca_pem.as_bytes()) {
        roots.add(ca.map_err(|_| TlsError("client CA"))?).map_err(|_| TlsError("client CA"))?;
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots)).build().map_err(|_| TlsError("client verifier"))?;
    let cfg = ServerConfig::builder().with_client_cert_verifier(verifier).with_single_cert(certs, key)
        .map_err(|_| TlsError("server certificate and key"))?;
    Ok(Arc::new(cfg))
}

fn common_name(cert: &CertificateDer<'_>) -> Option<String> {
    let (_, x) = x509_parser::parse_x509_certificate(cert.as_ref()).ok()?;
    let cn = x.subject().iter_common_name().next()?.as_str().ok()?.to_owned();
    Some(cn)
}

pub async fn serve_mtls(listener: TcpListener, config: Arc<ServerConfig>, app: Router, shutdown: impl Future<Output = ()> + Send + 'static) {
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    tokio::pin!(shutdown);
    loop {
        let (tcp, _) = tokio::select! {
            _ = &mut shutdown => return,
            accepted = listener.accept() => match accepted { Ok(a) => a, Err(_) => continue },
        };
        let (acceptor, app) = (acceptor.clone(), app.clone());
        tokio::spawn(async move {
            let Ok(tls) = acceptor.accept(tcp).await else { return };
            let Some(cn) = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).and_then(common_name) else { return };
            let svc = hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
                req.extensions_mut().insert(PeerIdentity(cn.clone()));
                let mut app = app.clone();
                async move { app.call(req).await }
            });
            let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), svc).await;
        });
    }
}
```

`src/test_support.rs`:

```rust
//! An in-process key service over real mTLS, for fau-key-client's and fau-app's
//! integration tests. Compiled only with `feature = "test-support"`.

use std::sync::Arc;

use crate::api::{self, ApiState, Clock, Limits};
use crate::devcerts::{self, DevCerts};
use crate::store::Store;
use crate::tls::{self, TlsMaterial};
use crate::vault::initialise;

pub struct TestServer {
    pub base_url: String,
    pub ca_pem: String,
    pub client_identity_pem: String,
    pub root_key: String,
    pub state: ApiState,
    certs: DevCerts,
    _dir: tempfile::TempDir,
}

impl TestServer {
    pub fn identity_for(&self, cn: &str) -> String {
        let (cert, key) = devcerts::issue_client(&self.certs, cn);
        format!("{cert}{key}")
    }
}

pub async fn spawn(limits: Limits, unsealed: bool) -> TestServer {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(Store::open(&dir.path().join("keys.db")).await.expect("store"));
    let root_key = initialise(&store).await.expect("init").to_string();
    let clock: Clock = Arc::new(jiff::Timestamp::now);
    let state = ApiState::new(store, clock, limits);
    if unsealed {
        state.vault().unseal(&root_key).await.expect("unseal");
    }
    let certs = devcerts::generate(&["localhost".into()], "fau-app");
    let config = tls::server_config(&TlsMaterial {
        cert_pem: certs.server_cert_pem.clone(),
        key_pem: certs.server_key_pem.clone(),
        client_ca_pem: certs.ca_pem.clone(),
    }).expect("tls config");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(tls::serve_mtls(listener, config, api::router(state.clone()), std::future::pending()));
    TestServer {
        base_url: format!("https://localhost:{port}"),
        ca_pem: certs.ca_pem.clone(),
        client_identity_pem: format!("{}{}", certs.client_cert_pem, certs.client_key_pem),
        root_key,
        state,
        certs,
        _dir: dir,
    }
}
```

`src/lib.rs`: add

```rust
pub mod devcerts;
pub mod tls;
#[cfg(feature = "test-support")]
pub mod test_support;
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p fau-key-service --features test-support`
Expected: all earlier tests plus 4 tls tests pass. If `pem_slice_iter` is not found, enable
`rustls-pki-types = { version = "1", features = ["std"] }`.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/key-service backend/Cargo.lock
git commit -m "Serve the key service over mTLS, with dev certificates and a test server (#3506)"
```

---

### Task 7: The `fau-keys` binary: configuration, hardening, unseal, init, dev guard

**Files:**
- Create: `backend/crates/key-service/src/{config,hardening,unseal}.rs`
- Replace: `src/main.rs`
- Test: `backend/crates/key-service/tests/binary.rs`, plus unit tests in `config.rs`

**Interfaces:**
- Consumes: everything above.
- Produces the process contract:

| Invocation | Behaviour |
|---|---|
| `fau-keys serve` | Reads config; applies hardening; opens the store; starts **sealed**; serves the API (mTLS), metrics and unseal listeners; exits 0 on SIGTERM |
| `fau-keys init --store PATH` | Prints the root key once to stdout and a warning to stderr; exits 1 with no stdout on an initialised store |
| `fau-keys unseal [--url http://127.0.0.1:8444]` | Reads the key without echo (from stdin when it is not a TTY); exits 0 when unsealed, 1 on a wrong key |
| `fau-keys dev-certs --out DIR [--server-name N]... [--client-cn CN]` | Writes the five PEM files |
| `fau-keys --version` | Prints `fau-keys <version>` and reads no configuration |

| Variable | Default | Rule |
|---|---|---|
| `FAU_KEYS_STORE` | required for `serve` | path to the SQLite file |
| `FAU_KEYS_API_BIND` | `0.0.0.0:8443` | |
| `FAU_KEYS_METRICS_BIND` | `0.0.0.0:9464` | |
| `FAU_KEYS_UNSEAL_BIND` | `127.0.0.1:8444` | **must be loopback**, otherwise `NotPermitted` |
| `FAU_KEYS_TLS_CERT`, `FAU_KEYS_TLS_KEY`, `FAU_KEYS_CLIENT_CA` | required for `serve` | PEM file paths |
| `FAU_KEYS_LEASE_IDLE_SECS` | `1800` | |
| `FAU_KEYS_CEILING` | `200` | |
| `FAU_KEYS_SESSION_PER_MINUTE` | `120` | |
| `FAU_KEYS_INSTANCE_PER_MINUTE` | `6000` | |
| `LOG_LEVEL` | `info` | |
| `FAU_KEYS_DEV_ROOT_KEY` | none | **without the `dev` feature, its presence is `NotPermitted`**. With `dev`, it is a passphrase: the root key is derived from it, the store initialised with it if empty, and unsealed at start |

- [ ] **Step 1: Write the failing tests**

`src/config.rs` unit tests:

```rust
#[cfg(test)]
mod tests {
    // `.err().unwrap()`, not `unwrap_err()`: ServeConfig deliberately has no Debug.
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + '_ {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned())
    }

    const BASE: &[(&str, &str)] = &[
        ("FAU_KEYS_STORE", "/data/keys.db"),
        ("FAU_KEYS_TLS_CERT", "/tls/tls.crt"),
        ("FAU_KEYS_TLS_KEY", "/tls/tls.key"),
        ("FAU_KEYS_CLIENT_CA", "/tls/ca.crt"),
    ];

    #[test]
    fn defaults_apply() {
        let c = ServeConfig::from_lookup(env(BASE)).unwrap();
        assert_eq!(c.unseal_bind.to_string(), "127.0.0.1:8444");
        assert_eq!(c.limits.ceiling, 200);
        assert!(c.dev_root_key.is_none());
    }

    #[test]
    fn a_non_loopback_unseal_bind_is_refused_by_name() {
        let mut e = BASE.to_vec();
        e.push(("FAU_KEYS_UNSEAL_BIND", "0.0.0.0:8444"));
        let err = ServeConfig::from_lookup(env(&e)).err().unwrap();
        assert_eq!(err.to_string(), "FAU_KEYS_UNSEAL_BIND: not permitted");
    }

    #[test]
    fn a_missing_variable_is_named_and_its_value_never_printed() {
        let err = ServeConfig::from_lookup(env(&BASE[1..])).err().unwrap();
        assert_eq!(err.to_string(), "FAU_KEYS_STORE: missing");
        let mut e = BASE.to_vec();
        e.push(("FAU_KEYS_CEILING", "lots-SECRETISH"));
        assert!(!ServeConfig::from_lookup(env(&e)).err().unwrap().to_string().contains("SECRETISH"));
    }

    #[cfg(not(feature = "dev"))]
    #[test]
    fn the_dev_root_key_is_refused_without_the_dev_feature() {
        let mut e = BASE.to_vec();
        e.push(("FAU_KEYS_DEV_ROOT_KEY", "anything"));
        assert_eq!(ServeConfig::from_lookup(env(&e)).err().unwrap().to_string(), "FAU_KEYS_DEV_ROOT_KEY: not permitted");
    }
}
```

`tests/binary.rs`:

```rust
//! The `fau-keys` process contract (spec §4.3–4.6, §6.4), through the real binary.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fau-keys");

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[cfg(not(feature = "dev"))]
#[test]
fn a_release_build_refuses_the_dev_root_key() {
    let out = Command::new(BIN).arg("serve").env("FAU_KEYS_DEV_ROOT_KEY", "x").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("FAU_KEYS_DEV_ROOT_KEY: not permitted"));
}

#[test]
fn init_prints_the_key_once_and_refuses_twice() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("keys.db");
    let first = Command::new(BIN).args(["init", "--store"]).arg(&store).output().unwrap();
    assert!(first.status.success());
    let key = String::from_utf8(first.stdout).unwrap();
    assert!(fau_key_service::root::RootKey::decode(&key).is_ok());
    assert!(String::from_utf8_lossy(&first.stderr).contains("Proton Pass"));
    let second = Command::new(BIN).args(["init", "--store"]).arg(&store).output().unwrap();
    assert!(!second.status.success());
    assert!(second.stdout.is_empty());
}

#[tokio::test]
async fn serve_starts_sealed_is_hardened_and_unseals_only_with_the_right_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("keys.db");
    let certs = dir.path().join("certs");
    assert!(Command::new(BIN).args(["dev-certs", "--out"]).arg(&certs).args(["--server-name", "localhost"]).status().unwrap().success());
    let key = String::from_utf8(Command::new(BIN).args(["init", "--store"]).arg(&store).output().unwrap().stdout).unwrap();
    let (api, metrics, unseal) = (free_port(), free_port(), free_port());
    let mut child = Command::new(BIN).arg("serve")
        .env("FAU_KEYS_STORE", &store)
        .env("FAU_KEYS_API_BIND", format!("127.0.0.1:{api}"))
        .env("FAU_KEYS_METRICS_BIND", format!("127.0.0.1:{metrics}"))
        .env("FAU_KEYS_UNSEAL_BIND", format!("127.0.0.1:{unseal}"))
        .env("FAU_KEYS_TLS_CERT", certs.join("server.pem"))
        .env("FAU_KEYS_TLS_KEY", certs.join("server-key.pem"))
        .env("FAU_KEYS_CLIENT_CA", certs.join("ca.pem"))
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let metrics_url = format!("http://127.0.0.1:{metrics}/metrics");
    let mut body = String::new();
    for _ in 0..100 {
        if let Ok(r) = reqwest::get(&metrics_url).await { body = r.text().await.unwrap(); break; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(body.contains("key_service_sealed 1"), "starts sealed");
    let limits = std::fs::read_to_string(format!("/proc/{}/limits", child.id())).unwrap();
    assert!(limits.lines().any(|l| l.starts_with("Max core file size") && l.split_whitespace().nth(4) == Some("0")), "RLIMIT_CORE is 0");

    let unseal_with = |k: String| {
        let mut p = Command::new(BIN).args(["unseal", "--url", &format!("http://127.0.0.1:{unseal}")])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        p.stdin.take().unwrap().write_all(k.as_bytes()).unwrap();
        p.wait_with_output().unwrap()
    };
    let wrong = fau_key_service::root::RootKey::generate().unwrap().encode().to_string();
    assert!(!unseal_with(wrong).status.success());
    assert!(reqwest::get(&metrics_url).await.unwrap().text().await.unwrap().contains("key_service_unseal_failures_total 1"));
    assert!(unseal_with(key.clone()).status.success());
    assert!(reqwest::get(&metrics_url).await.unwrap().text().await.unwrap().contains("key_service_sealed 0"));

    nix_kill_term(child.id());
    let status = child.wait().unwrap();
    assert!(status.success(), "SIGTERM exits 0");
    let logs = String::from_utf8_lossy(&child.stdout.take().map(|mut s| { let mut v = Vec::new(); std::io::Read::read_to_end(&mut s, &mut v).unwrap(); v }).unwrap_or_default()).to_string();
    assert!(!logs.contains(key.trim()), "the root key is never logged");
}

fn nix_kill_term(pid: u32) {
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
}
```

(`libc` is a normal dependency of this crate, so tests can use it.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-key-service --lib config && cargo test -p fau-key-service --test binary`
Expected: compile errors, because `config` and the subcommands are missing.

- [ ] **Step 3: Implement `src/config.rs`**

```rust
//! `fau-keys serve` configuration. Errors name the variable, never the value.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use jiff::SignedDuration;

use crate::api::Limits;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem { Missing, Invalid, NotPermitted }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError { pub var: &'static str, pub problem: Problem }

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = match self.problem { Problem::Missing => "missing", Problem::Invalid => "invalid", Problem::NotPermitted => "not permitted" };
        write!(f, "{}: {p}", self.var)
    }
}

impl std::error::Error for ConfigError {}

pub struct ServeConfig {
    pub store: PathBuf,
    pub api_bind: SocketAddr,
    pub metrics_bind: SocketAddr,
    pub unseal_bind: SocketAddr,
    pub tls_cert: PathBuf,
    pub tls_key: PathBuf,
    pub client_ca: PathBuf,
    pub limits: Limits,
    pub log_level: String,
    /// Only ever `Some` in a `dev` build.
    pub dev_root_key: Option<zeroize::Zeroizing<String>>,
}

impl ServeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let req = |var: &'static str| get(var).filter(|v| !v.is_empty()).ok_or(ConfigError { var, problem: Problem::Missing });
        fn parse<T: std::str::FromStr>(var: &'static str, v: Option<String>, default: T) -> Result<T, ConfigError> {
            match v {
                None => Ok(default),
                Some(s) => s.parse().map_err(|_| ConfigError { var, problem: Problem::Invalid }),
            }
        }
        let unseal_bind: SocketAddr = parse("FAU_KEYS_UNSEAL_BIND", get("FAU_KEYS_UNSEAL_BIND"), "127.0.0.1:8444".parse().unwrap())?;
        if !unseal_bind.ip().is_loopback() {
            return Err(ConfigError { var: "FAU_KEYS_UNSEAL_BIND", problem: Problem::NotPermitted });
        }
        let dev_root_key = get("FAU_KEYS_DEV_ROOT_KEY");
        #[cfg(not(feature = "dev"))]
        if dev_root_key.is_some() {
            return Err(ConfigError { var: "FAU_KEYS_DEV_ROOT_KEY", problem: Problem::NotPermitted });
        }
        let d = Limits::default();
        Ok(Self {
            store: req("FAU_KEYS_STORE")?.into(),
            api_bind: parse("FAU_KEYS_API_BIND", get("FAU_KEYS_API_BIND"), "0.0.0.0:8443".parse().unwrap())?,
            metrics_bind: parse("FAU_KEYS_METRICS_BIND", get("FAU_KEYS_METRICS_BIND"), "0.0.0.0:9464".parse().unwrap())?,
            unseal_bind,
            tls_cert: req("FAU_KEYS_TLS_CERT")?.into(),
            tls_key: req("FAU_KEYS_TLS_KEY")?.into(),
            client_ca: req("FAU_KEYS_CLIENT_CA")?.into(),
            limits: Limits {
                idle: SignedDuration::from_secs(parse("FAU_KEYS_LEASE_IDLE_SECS", get("FAU_KEYS_LEASE_IDLE_SECS"), d.idle.as_secs())?),
                ceiling: parse("FAU_KEYS_CEILING", get("FAU_KEYS_CEILING"), d.ceiling)?,
                session_per_minute: parse("FAU_KEYS_SESSION_PER_MINUTE", get("FAU_KEYS_SESSION_PER_MINUTE"), d.session_per_minute)?,
                instance_per_minute: parse("FAU_KEYS_INSTANCE_PER_MINUTE", get("FAU_KEYS_INSTANCE_PER_MINUTE"), d.instance_per_minute)?,
            },
            log_level: get("LOG_LEVEL").unwrap_or_else(|| "info".into()),
            dev_root_key: dev_root_key.map(zeroize::Zeroizing::new),
        })
    }
}
```

The dev-key check comes before `FAU_KEYS_STORE` is read, so `a_release_build_refuses_the_dev_root_key`
fails for the right reason even with no other variables set. Keep that order.

- [ ] **Step 4: Implement `src/hardening.rs` and `src/unseal.rs`**

```rust
//! Process hardening at start (spec §4.6): no core dumps, not ptrace-attachable by
//! same-uid processes, so the root key cannot reach disk through a crash.

pub fn apply() -> Result<(), &'static str> {
    // SAFETY: plain syscalls with constant arguments; no memory is shared with the kernel
    // beyond the `rlimit` struct on our stack.
    unsafe {
        if libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0 {
            return Err("prctl(PR_SET_DUMPABLE)");
        }
        let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::setrlimit(libc::RLIMIT_CORE, &none) != 0 {
            return Err("setrlimit(RLIMIT_CORE)");
        }
    }
    Ok(())
}
```

`src/unseal.rs`:

```rust
//! The loopback-only unseal listener (spec §4.5). Plain HTTP: it is reachable only
//! through `kubectl port-forward` into the pod's own network namespace.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use fau_key_protocol::UnsealRequest;
use serde_json::{json, Value};

use crate::api::ApiState;
use crate::metrics::inc;
use crate::vault::UnsealError;

async fn unseal(State(s): State<ApiState>, Json(req): Json<UnsealRequest>) -> (StatusCode, Json<Value>) {
    match s.vault().unseal(&req.root_key).await {
        Ok(()) => {
            tracing::warn!("key service unsealed");
            (StatusCode::OK, Json(json!({ "sealed": false })))
        }
        Err(e) => {
            inc(&s.metrics().unseal_failures);
            tracing::error!(reason = ?e, "unseal refused");
            let status = match e {
                UnsealError::NotInitialised => StatusCode::CONFLICT,
                UnsealError::Store => StatusCode::INTERNAL_SERVER_ERROR,
                UnsealError::BadFormat | UnsealError::WrongKey => StatusCode::UNAUTHORIZED,
            };
            (status, Json(json!({ "error": "refused" })))
        }
    }
}

pub fn router(state: ApiState) -> Router {
    Router::new().route(concat!("/", "unseal"), post(unseal)).with_state(state)
}
```

`UnsealError` derives `Debug` with variant names only, so `reason = ?e` carries no key material.

- [ ] **Step 5: Implement `src/main.rs`**

```rust
//! `fau-keys`: the FAU key service (docs/key-service-design.md).

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use fau_key_service::api::{self, ApiState, Clock};
use fau_key_service::config::ServeConfig;
use fau_key_service::store::Store;
use fau_key_service::tls::{self, TlsMaterial};
use fau_key_service::{devcerts, hardening, unseal, vault};

#[derive(Parser)]
#[command(name = "fau-keys", disable_version_flag = true)]
struct Cli {
    #[arg(long, global = true)]
    version: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve the key service. Starts sealed.
    Serve,
    /// Generate the root key for an empty store and print it once.
    Init { #[arg(long)] store: PathBuf },
    /// Send the root key to a running service's loopback unseal listener.
    Unseal { #[arg(long, default_value = "http://127.0.0.1:8444")] url: String },
    /// Write a throwaway CA, server and client certificate for development.
    DevCerts {
        #[arg(long)] out: PathBuf,
        #[arg(long = "server-name", default_values_t = vec!["keys".to_string(), "localhost".to_string()])] server_names: Vec<String>,
        #[arg(long, default_value = "fau-app")] client_cn: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.version {
        println!("fau-keys {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    match cli.command {
        Some(Cmd::Serve) => rt.block_on(serve()),
        Some(Cmd::Init { store }) => rt.block_on(init(store)),
        Some(Cmd::Unseal { url }) => rt.block_on(send_unseal(url)),
        Some(Cmd::DevCerts { out, server_names, client_cn }) => {
            match devcerts::generate(&server_names, &client_cn).write_to(&out) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => { eprintln!("fau-keys: could not write certificates: {e}"); ExitCode::FAILURE }
            }
        }
        None => { eprintln!("fau-keys: expected serve, init, unseal or dev-certs"); ExitCode::FAILURE }
    }
}

async fn serve() -> ExitCode {
    let cfg = match ServeConfig::from_env() {
        Ok(c) => c,
        Err(e) => { eprintln!("fau-keys: {e}"); return ExitCode::FAILURE; }
    };
    if let Err(what) = hardening::apply() {
        eprintln!("fau-keys: hardening failed: {what}");
        return ExitCode::FAILURE;
    }
    tracing_subscriber::fmt().json().with_env_filter(tracing_subscriber::EnvFilter::new(&cfg.log_level)).init();
    let store = match Store::open(&cfg.store).await {
        Ok(s) => Arc::new(s),
        Err(e) => { tracing::error!(error = %e, "cannot open the key store"); return ExitCode::FAILURE; }
    };
    let read = |p: &PathBuf| std::fs::read_to_string(p);
    let material = match (read(&cfg.tls_cert), read(&cfg.tls_key), read(&cfg.client_ca)) {
        (Ok(cert_pem), Ok(key_pem), Ok(client_ca_pem)) => TlsMaterial { cert_pem, key_pem, client_ca_pem },
        _ => { tracing::error!("cannot read the TLS files"); return ExitCode::FAILURE; }
    };
    let tls_config = match tls::server_config(&material) {
        Ok(c) => c,
        Err(e) => { tracing::error!(error = %e, "invalid TLS material"); return ExitCode::FAILURE; }
    };
    let clock: Clock = Arc::new(jiff::Timestamp::now);
    let state = ApiState::new(store.clone(), clock, cfg.limits);
    #[cfg(feature = "dev")]
    if let Some(dev) = &cfg.dev_root_key {
        // Config refuses the variable in any build without `dev`. The value is a
        // passphrase, not a root key, so compose can carry an obviously fake default.
        let key = fau_key_service::root::RootKey::from_dev_passphrase(dev);
        if !store.has_canary().await.unwrap_or(false) {
            store.write_canary(&key).await.expect("canary");
        }
        state.vault().unseal(&key.encode()).await.expect("dev unseal");
        tracing::warn!("DEV MODE: unsealed from FAU_KEYS_DEV_ROOT_KEY");
    }
    let bind = |a| async move { tokio::net::TcpListener::bind(a).await };
    let (api_l, metrics_l, unseal_l) = match (bind(cfg.api_bind).await, bind(cfg.metrics_bind).await, bind(cfg.unseal_bind).await) {
        (Ok(a), Ok(m), Ok(u)) => (a, m, u),
        _ => { tracing::error!("cannot bind a listener"); return ExitCode::FAILURE; }
    };
    tracing::info!(api = %cfg.api_bind, metrics = %cfg.metrics_bind, unseal = %cfg.unseal_bind, sealed = state.vault().is_sealed(), "fau-keys listening");
    let (tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let stop = |tx: &tokio::sync::broadcast::Sender<()>| { let mut rx = tx.subscribe(); async move { let _ = rx.recv().await; } };
    let api_task = tokio::spawn(tls::serve_mtls(api_l, tls_config, api::router(state.clone()), stop(&tx)));
    let m = axum::serve(metrics_l, api::metrics_router(state.clone())).with_graceful_shutdown(stop(&tx));
    let u = axum::serve(unseal_l, unseal::router(state.clone())).with_graceful_shutdown(stop(&tx));
    let metrics_task = tokio::spawn(async move { let _ = m.await; });
    let unseal_task = tokio::spawn(async move { let _ = u.await; });
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
    tokio::select! { _ = term.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
    let _ = tx.send(());
    let _ = tokio::join!(api_task, metrics_task, unseal_task);
    tracing::info!("fau-keys stopped");
    ExitCode::SUCCESS
}

async fn init(path: PathBuf) -> ExitCode {
    let store = match Store::open(&path).await {
        Ok(s) => s,
        Err(e) => { eprintln!("fau-keys: cannot open the store: {e}"); return ExitCode::FAILURE; }
    };
    match vault::initialise(&store).await {
        Ok(key) => {
            eprintln!("fau-keys: the root key follows on stdout. Store it in Proton Pass now; it is shown only once and never stored.");
            print!("{}", &*key);
            ExitCode::SUCCESS
        }
        Err(vault::InitError::AlreadyInitialised) => { eprintln!("fau-keys: this store is already initialised"); ExitCode::FAILURE }
        Err(vault::InitError::Store) => { eprintln!("fau-keys: the store could not be written"); ExitCode::FAILURE }
    }
}

async fn send_unseal(url: String) -> ExitCode {
    let key = zeroize::Zeroizing::new(if std::io::stdin().is_terminal() {
        match rpassword::prompt_password("Root key: ") { Ok(k) => k, Err(_) => return ExitCode::FAILURE }
    } else {
        let mut s = String::new();
        if std::io::stdin().read_to_string(&mut s).is_err() { return ExitCode::FAILURE; }
        s
    });
    let body = fau_key_protocol::UnsealRequest { root_key: key.trim().to_owned() };
    match reqwest::Client::new().post(format!("{url}/unseal")).json(&body).send().await {
        Ok(r) if r.status().is_success() => { eprintln!("fau-keys: unsealed"); ExitCode::SUCCESS }
        Ok(r) => { eprintln!("fau-keys: refused ({})", r.status()); ExitCode::FAILURE }
        Err(_) => { eprintln!("fau-keys: cannot reach {url}"); ExitCode::FAILURE }
    }
}
```

Add to `src/root.rs`, with a test, so a dev store is keyed by a passphrase and compose can
carry a default that is obviously not a real key:

```rust
impl RootKey {
    /// Dev builds only (spec §6.4): SHA-256 of a fixed prefix and the passphrase.
    #[cfg(feature = "dev")]
    pub fn from_dev_passphrase(passphrase: &str) -> Self {
        let d = Sha256::digest(format!("fau-keys-dev|{passphrase}").as_bytes());
        let mut k = Zeroizing::new([0u8; 32]);
        k.copy_from_slice(&d);
        Self(k)
    }
}

#[cfg(all(test, feature = "dev"))]
mod dev_tests {
    use super::*;
    #[test]
    fn a_dev_passphrase_is_deterministic_and_distinct() {
        assert_eq!(RootKey::from_dev_passphrase("a").expose(), RootKey::from_dev_passphrase("a").expose());
        assert_ne!(RootKey::from_dev_passphrase("a").expose(), RootKey::from_dev_passphrase("b").expose());
    }
}
```

Add `pub mod config; pub mod hardening; pub mod unseal;` to `src/lib.rs`. The `unseal` client
uses plain `http://` to the loopback listener; the `reqwest` `rustls` feature does not stop
plain HTTP.

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p fau-key-service --features test-support`
Expected: every suite passes, including `binary` (3 tests without the `dev` feature).
Then: `cargo test -p fau-key-service --features dev --lib root`
Expected: the dev passphrase test passes. The config test for the dev guard is compiled out
under `dev`, which is correct.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/key-service backend/Cargo.lock
git commit -m "Add the fau-keys binary: serve sealed, init, unseal, dev-certs and the dev guard (#3506)"
```

---

### Task 8: `fau-key-client`: the mTLS client and the session-held `KeyCache`

**Files:**
- Replace the stub: `backend/crates/key-client/Cargo.toml`, `src/lib.rs`
- Create: `src/client.rs`, `src/cache.rs`
- Test: `backend/crates/key-client/tests/cache.rs`, against `fau_key_service::test_support::spawn`

**Interfaces:**
- Consumes: `fau-key-protocol`, `fau-crypto` (`DataKey`, `KeyId`, `SealingPublicKey`), and in tests `fau-key-service` with `test-support`.
- Produces (for Tasks 9–10 and every future feature):
  - `ClientConfig { base_url: String, ca_pem: String, identity_pem: String, instance_id: String, timeout: Duration }`.
  - `KeyClient::new(ClientConfig) -> Result<KeyClient, KeyError>`.
  - `Who { account_id: Uuid, session_id: Uuid }`.
  - `LeasedKey { key_id: KeyId, key: DataKey, lease_id: Uuid, idle: Duration }`.
  - Methods:
    - `create_fau(fau, &Who) -> Result<SealingPublicKey, KeyError>`;
    - `public_key(fau) -> Result<SealingPublicKey, KeyError>`;
    - `unwrap(fau, KeyClass, Option<&str>, &Who) -> Result<LeasedKey, KeyError>`;
    - `create_key(fau, KeyClass, &str, &Who) -> Result<LeasedKey, KeyError>`;
    - `renew(lease) -> Result<(), KeyError>`;
    - `release(lease) -> Result<(), KeyError>`;
    - `destroy_key(fau, KeyClass, &str, &Who) -> Result<(), KeyError>`;
    - `destroy_fau(fau, &Who) -> Result<String /*due_at*/, KeyError>`.
  - `KeyError { Sealed, CeilingReached, RateLimited, NotFound, Refused, Invalid, Unavailable }`.
  - `KeyCache::new(client: Arc<KeyClient>, clock: Arc<dyn Fn() -> jiff::Timestamp + Send + Sync>)`, with methods:
    - `get(&Who, fau, KeyClass, Option<&str>) -> Result<(KeyId, DataKey), KeyError>`, which counts as activity;
    - `touch_session(session: Uuid)`, called by middleware for real user requests only;
    - `release_session(session)`;
    - `release_document(session, fau, doc: Uuid)`;
    - `sweep() -> usize`, which releases entries idle past their lease timeout and is **not** activity;
    - `len() -> usize`.

**Renewal rule:** `touch_session` sets `last_activity = now` on the session's entries. For each
entry last renewed more than `idle / 3` ago, it also calls `renew`. That bounds renewals to about
three per lease per idle period, however chatty the user is. An entry whose `renew` returns
`NotFound` (it expired at the service) is dropped from the cache.

- [ ] **Step 1: Write the failing tests** (`tests/cache.rs`)

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fau_key_client::{ClientConfig, KeyCache, KeyClient, KeyError, Who};
use fau_key_protocol::KeyClass;
use fau_key_service::api::Limits;
use fau_key_service::test_support::{spawn, TestServer};
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

fn client_for(srv: &TestServer) -> Arc<KeyClient> {
    Arc::new(KeyClient::new(ClientConfig {
        base_url: srv.base_url.clone(),
        ca_pem: srv.ca_pem.clone(),
        identity_pem: srv.client_identity_pem.clone(),
        instance_id: "app-0".into(),
        timeout: Duration::from_secs(5),
    }).unwrap())
}

struct Clock(Arc<Mutex<Timestamp>>);
impl Clock {
    fn new() -> (Self, Arc<dyn Fn() -> Timestamp + Send + Sync>) {
        let t = Arc::new(Mutex::new(Timestamp::now()));
        let t2 = t.clone();
        (Self(t), Arc::new(move || *t2.lock().unwrap()))
    }
    fn advance(&self, mins: i64) {
        let mut t = self.0.lock().unwrap();
        *t = *t + SignedDuration::from_mins(mins);
    }
}

fn who() -> Who { Who { account_id: Uuid::now_v7(), session_id: Uuid::now_v7() } }

fn live_leases(srv: &TestServer) -> u64 {
    srv.state.metrics_text().lines().find_map(|l| l.strip_prefix("key_service_live_leases ")).unwrap().parse().unwrap()
}

#[tokio::test]
async fn a_cached_key_is_fetched_once_per_session() {
    let srv = spawn(Limits::default(), true).await;
    let client = client_for(&srv);
    let (_c, clock) = Clock::new();
    let cache = KeyCache::new(client.clone(), clock);
    let w = who();
    let fau = Uuid::now_v7();
    client.create_fau(fau, &w).await.unwrap();
    let (id1, k1) = cache.get(&w, fau, KeyClass::Record, None).await.unwrap();
    let (id2, k2) = cache.get(&w, fau, KeyClass::Record, None).await.unwrap();
    assert_eq!((id1, k1.expose()), (id2, k2.expose()));
    assert!(srv.state.metrics_text().contains("key_service_unwraps_total 1"));
    assert_eq!(cache.len(), 1);
}

#[tokio::test]
async fn an_idle_session_is_swept_and_its_lease_released_but_sweeping_is_not_activity() {
    let srv = spawn(Limits::default(), true).await;
    let client = client_for(&srv);
    let (c, clock) = Clock::new();
    let cache = KeyCache::new(client.clone(), clock);
    let w = who();
    let fau = Uuid::now_v7();
    client.create_fau(fau, &w).await.unwrap();
    cache.get(&w, fau, KeyClass::Record, None).await.unwrap();
    assert_eq!(live_leases(&srv), 1);
    c.advance(29);
    assert_eq!(cache.sweep().await, 0);
    c.advance(2);
    assert_eq!(cache.sweep().await, 1, "idle 31 minutes: the earlier sweep did not count as activity");
    assert_eq!(cache.len(), 0);
    assert_eq!(live_leases(&srv), 0, "released at the service");
}

#[tokio::test]
async fn real_activity_keeps_a_session_alive() {
    let srv = spawn(Limits::default(), true).await;
    let client = client_for(&srv);
    let (c, clock) = Clock::new();
    let cache = KeyCache::new(client.clone(), clock);
    let w = who();
    let fau = Uuid::now_v7();
    client.create_fau(fau, &w).await.unwrap();
    cache.get(&w, fau, KeyClass::Record, None).await.unwrap();
    for _ in 0..4 {
        c.advance(20);
        cache.touch_session(w.session_id).await;
        assert_eq!(cache.sweep().await, 0);
    }
    assert_eq!(cache.len(), 1);
}

#[tokio::test]
async fn logout_releases_everything_for_the_session_only() {
    let srv = spawn(Limits::default(), true).await;
    let client = client_for(&srv);
    let (_c, clock) = Clock::new();
    let cache = KeyCache::new(client.clone(), clock);
    let (a, b) = (who(), who());
    let fau = Uuid::now_v7();
    client.create_fau(fau, &a).await.unwrap();
    cache.get(&a, fau, KeyClass::Record, None).await.unwrap();
    cache.get(&a, fau, KeyClass::Sealing, None).await.unwrap();
    cache.get(&b, fau, KeyClass::Record, None).await.unwrap();
    cache.release_session(a.session_id).await;
    assert_eq!(cache.len(), 1);
    assert_eq!(live_leases(&srv), 1);
}

#[tokio::test]
async fn sealed_and_ceiling_are_typed_errors() {
    let sealed = spawn(Limits::default(), false).await;
    let (_c, clock) = Clock::new();
    let cache = KeyCache::new(client_for(&sealed), clock.clone());
    assert_eq!(cache.get(&who(), Uuid::now_v7(), KeyClass::Record, None).await.unwrap_err(), KeyError::Sealed);

    let tight = spawn(Limits { ceiling: 1, ..Limits::default() }, true).await;
    let client = client_for(&tight);
    let cache = KeyCache::new(client.clone(), clock);
    let w = who();
    let (f1, f2) = (Uuid::now_v7(), Uuid::now_v7());
    client.create_fau(f1, &w).await.unwrap();
    client.create_fau(f2, &w).await.unwrap();
    cache.get(&w, f1, KeyClass::Record, None).await.unwrap();
    assert_eq!(cache.get(&w, f2, KeyClass::Record, None).await.unwrap_err(), KeyError::CeilingReached);
}

#[tokio::test]
async fn the_cache_debug_prints_no_key() {
    let srv = spawn(Limits::default(), true).await;
    let client = client_for(&srv);
    let (_c, clock) = Clock::new();
    let cache = KeyCache::new(client.clone(), clock);
    let w = who();
    let fau = Uuid::now_v7();
    client.create_fau(fau, &w).await.unwrap();
    let (_, k) = cache.get(&w, fau, KeyClass::Record, None).await.unwrap();
    let encoded = fau_key_protocol::encode_key(k.expose());
    assert!(!format!("{cache:?}").contains(&encoded));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-key-client`
Expected: compile errors.

- [ ] **Step 3: Implement**

`Cargo.toml`:

```toml
[package]
name = "fau-key-client"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

[dependencies]
fau-crypto = { path = "../crypto" }
fau-key-protocol = { path = "../key-protocol" }
jiff = { workspace = true }
reqwest = { version = "0.13", default-features = false, features = ["rustls", "json"] }
serde = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
fau-key-service = { path = "../key-service", features = ["test-support"] }
```

`src/client.rs`:

```rust
//! The backend's mTLS client for the key service (spec §5.2).

use std::time::Duration;

use fau_crypto::{DataKey, KeyId, SealingPublicKey};
use fau_key_protocol::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("the key service is sealed")] Sealed,
    #[error("the key service's concurrency ceiling is reached")] CeilingReached,
    #[error("the key service rate limit is reached")] RateLimited,
    #[error("no such key")] NotFound,
    #[error("the key service refused the operation")] Refused,
    #[error("the key service rejected the request as malformed")] Invalid,
    #[error("the key service is unavailable")] Unavailable,
}

pub struct ClientConfig {
    pub base_url: String,
    pub ca_pem: String,
    pub identity_pem: String,
    pub instance_id: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct Who { pub account_id: Uuid, pub session_id: Uuid }

#[derive(Debug, Clone)]
pub struct LeasedKey { pub key_id: KeyId, pub key: DataKey, pub lease_id: Uuid, pub idle: Duration }

pub struct KeyClient { http: reqwest::Client, base: String, instance_id: String }

impl std::fmt::Debug for KeyClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyClient").field("base", &self.base).field("instance_id", &self.instance_id).finish()
    }
}

impl KeyClient {
    pub fn new(cfg: ClientConfig) -> Result<Self, KeyError> {
        let ca = reqwest::Certificate::from_pem(cfg.ca_pem.as_bytes()).map_err(|_| KeyError::Invalid)?;
        let id = reqwest::Identity::from_pem(cfg.identity_pem.as_bytes()).map_err(|_| KeyError::Invalid)?;
        let http = reqwest::Client::builder().tls_certs_only([ca]).identity(id).timeout(cfg.timeout).build().map_err(|_| KeyError::Invalid)?;
        Ok(Self { http, base: cfg.base_url.trim_end_matches('/').to_owned(), instance_id: cfg.instance_id })
    }

    fn caller(&self, who: &Who) -> Caller {
        Caller { account_id: who.account_id, session_id: who.session_id, instance_id: self.instance_id.clone() }
    }

    async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> Result<T, KeyError> {
        let res = req.send().await.map_err(|_| KeyError::Unavailable)?;
        if res.status().is_success() {
            return res.json().await.map_err(|_| KeyError::Unavailable);
        }
        let code = res.json::<ErrorBody>().await.map(|b| b.error).unwrap_or(ErrorCode::Internal);
        Err(match code {
            ErrorCode::Sealed => KeyError::Sealed,
            ErrorCode::CeilingReached => KeyError::CeilingReached,
            ErrorCode::RateLimited => KeyError::RateLimited,
            ErrorCode::NotFound => KeyError::NotFound,
            ErrorCode::Refused => KeyError::Refused,
            ErrorCode::BadRequest => KeyError::Invalid,
            ErrorCode::Internal => KeyError::Unavailable,
        })
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T, KeyError> {
        self.send(self.http.post(format!("{}{path}", self.base)).json(body)).await
    }

    fn leased(r: KeyResponse) -> Result<LeasedKey, KeyError> {
        let key = decode_key(&r.key).ok_or(KeyError::Unavailable)?;
        Ok(LeasedKey { key_id: KeyId::new(r.key_id), key: DataKey::from_bytes(*key), lease_id: r.lease_id, idle: Duration::from_secs(r.idle_timeout_secs) })
    }

    fn public(r: PublicKeyResponse) -> Result<SealingPublicKey, KeyError> {
        let pk = decode_key(&r.public_key).ok_or(KeyError::Unavailable)?;
        Ok(SealingPublicKey::from_bytes(*pk))
    }

    pub async fn create_fau(&self, fau: Uuid, who: &Who) -> Result<SealingPublicKey, KeyError> {
        Self::public(self.post("/v1/faus", &CreateFauRequest { fau_id: fau, caller: self.caller(who) }).await?)
    }

    pub async fn public_key(&self, fau: Uuid) -> Result<SealingPublicKey, KeyError> {
        Self::public(self.send(self.http.get(format!("{}/v1/faus/{fau}/public-key", self.base))).await?)
    }

    pub async fn unwrap(&self, fau: Uuid, class: KeyClass, subject: Option<&str>, who: &Who) -> Result<LeasedKey, KeyError> {
        Self::leased(self.post("/v1/keys/unwrap", &UnwrapRequest { fau_id: fau, class, subject: subject.map(str::to_owned), caller: self.caller(who) }).await?)
    }

    pub async fn create_key(&self, fau: Uuid, class: KeyClass, subject: &str, who: &Who) -> Result<LeasedKey, KeyError> {
        Self::leased(self.post("/v1/keys/create", &CreateKeyRequest { fau_id: fau, class, subject: subject.to_owned(), caller: self.caller(who) }).await?)
    }

    pub async fn renew(&self, lease: Uuid) -> Result<(), KeyError> {
        let _: LeaseResponse = self.post("/v1/leases/renew", &LeaseRequest { lease_id: lease, instance_id: self.instance_id.clone() }).await?;
        Ok(())
    }

    pub async fn release(&self, lease: Uuid) -> Result<(), KeyError> {
        let _: serde_json::Value = self.post("/v1/leases/release", &LeaseRequest { lease_id: lease, instance_id: self.instance_id.clone() }).await?;
        Ok(())
    }

    pub async fn destroy_key(&self, fau: Uuid, class: KeyClass, subject: &str, who: &Who) -> Result<(), KeyError> {
        let _: serde_json::Value = self.post("/v1/keys/destroy", &DestroyKeyRequest { fau_id: fau, class, subject: subject.to_owned(), caller: self.caller(who) }).await?;
        Ok(())
    }

    pub async fn destroy_fau(&self, fau: Uuid, who: &Who) -> Result<String, KeyError> {
        let r: DestroyFauResponse = self.post(&format!("/v1/faus/{fau}/destroy"), &DestroyFauRequest { caller: self.caller(who) }).await?;
        Ok(r.due_at)
    }
}
```

Add `serde_json = { workspace = true }` to the client's `[dependencies]`, since `release` and
`destroy_key` parse `{}`.

`src/cache.rs`:

```rust
//! Session-held keys (ADR-003 decision 5a; spec §5.3). Memory only; zeroised when
//! dropped (`DataKey` is `Zeroizing`). Only real user requests count as activity.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fau_crypto::{DataKey, KeyId};
use fau_key_protocol::KeyClass;
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

use crate::client::{KeyClient, KeyError, Who};

pub type CacheClock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Slot { session: Uuid, fau: Uuid, class: KeyClass, subject: Option<String> }

struct Entry { key_id: KeyId, key: DataKey, lease: Uuid, idle: SignedDuration, last_activity: Timestamp, last_renewed: Timestamp }

pub struct KeyCache {
    client: Arc<KeyClient>,
    clock: CacheClock,
    entries: Mutex<HashMap<Slot, Entry>>,
}

impl std::fmt::Debug for KeyCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyCache").field("entries", &self.len()).finish()
    }
}

impl KeyCache {
    pub fn new(client: Arc<KeyClient>, clock: CacheClock) -> Self {
        Self { client, clock, entries: Mutex::new(HashMap::new()) }
    }

    pub fn len(&self) -> usize {
        self.entries.lock().expect("cache lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub async fn get(&self, who: &Who, fau: Uuid, class: KeyClass, subject: Option<&str>) -> Result<(KeyId, DataKey), KeyError> {
        let slot = Slot { session: who.session_id, fau, class, subject: subject.map(str::to_owned) };
        let now = (self.clock)();
        if let Some(e) = self.entries.lock().expect("cache lock").get_mut(&slot) {
            e.last_activity = now;
            return Ok((e.key_id, e.key.clone()));
        }
        let leased = self.client.unwrap(fau, class, subject, who).await?;
        let idle = SignedDuration::try_from(leased.idle).unwrap_or(SignedDuration::from_mins(30));
        let out = (leased.key_id, leased.key.clone());
        let previous = self.entries.lock().expect("cache lock").insert(slot, Entry {
            key_id: leased.key_id, key: leased.key, lease: leased.lease_id, idle, last_activity: now, last_renewed: now,
        });
        if let Some(p) = previous {
            let _ = self.client.release(p.lease).await;
        }
        Ok(out)
    }

    pub async fn touch_session(&self, session: Uuid) {
        let now = (self.clock)();
        let due: Vec<(Slot, Uuid)> = {
            let mut map = self.entries.lock().expect("cache lock");
            map.iter_mut()
                .filter(|(s, _)| s.session == session)
                .filter_map(|(s, e)| {
                    e.last_activity = now;
                    (now.duration_since(e.last_renewed) > e.idle / 3).then(|| (s.clone(), e.lease))
                })
                .collect()
        };
        for (slot, lease) in due {
            match self.client.renew(lease).await {
                Ok(()) => { if let Some(e) = self.entries.lock().expect("cache lock").get_mut(&slot) { e.last_renewed = now; } }
                Err(KeyError::NotFound) => { self.entries.lock().expect("cache lock").remove(&slot); }
                Err(_) => {}
            }
        }
    }

    async fn release_where(&self, pred: impl Fn(&Slot, &Entry) -> bool) -> usize {
        let gone: Vec<Entry> = {
            let mut map = self.entries.lock().expect("cache lock");
            let keys: Vec<Slot> = map.iter().filter(|(s, e)| pred(s, e)).map(|(s, _)| s.clone()).collect();
            keys.into_iter().filter_map(|k| map.remove(&k)).collect()
        };
        for e in &gone {
            if let Err(err) = self.client.release(e.lease).await {
                tracing::warn!(error = %err, "key lease release failed; it will expire at the key service");
            }
        }
        gone.len()
    }

    pub async fn release_session(&self, session: Uuid) {
        self.release_where(|s, _| s.session == session).await;
    }

    pub async fn release_document(&self, session: Uuid, fau: Uuid, doc: Uuid) {
        let doc = doc.to_string();
        self.release_where(|s, _| s.session == session && s.fau == fau && s.class == KeyClass::Document && s.subject.as_deref() == Some(doc.as_str())).await;
    }

    /// Not activity: reads `last_activity`, never writes it.
    pub async fn sweep(&self) -> usize {
        let now = (self.clock)();
        self.release_where(|_, e| now.duration_since(e.last_activity) >= e.idle).await
    }
}
```

`src/lib.rs`:

```rust
//! The backend's side of the key service (docs/key-service-design.md §5.2–5.3).

mod cache;
mod client;

pub use cache::{CacheClock, KeyCache};
pub use client::{ClientConfig, KeyClient, KeyError, LeasedKey, Who};
```

In the sweep test, 29 + 2 = 31 minutes, which is at least the 30-minute idle, so the entry is
released. In the activity test, every touch at 20-minute intervals is past `idle / 3` = 10
minutes, so each touch renews at the service too, which exercises `renew`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p fau-key-client`
Expected: 6 passed. A test where the service's own lease expires on its real clock cannot happen,
because the tests take seconds. The service uses a real clock, the cache the test clock.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/key-client backend/Cargo.lock
git commit -m "Add fau-key-client: the mTLS client and the session-held key cache (#3506)"
```

---

### Task 9: The first consumers: the sealed access-request message and the invitation message

This ends #3418's "accept no message until the key service exists" deferral (spec §5.5).
Persistence never sees plaintext: it takes `fau_crypto::Sealed` / `Ciphertext`, which the HTTP
layer (#3417) produces with the key service's keys. The message carries the id of the row it is
bound to, because the associated data names that row (Task 1), and persistence uses that id for
the new row.

**Files:**
- Modify: `backend/crates/persistence/Cargo.toml` (add `fau-crypto = { path = "../crypto" }`)
- Modify: `backend/crates/persistence/src/membership/requests.rs`
- Modify: `backend/crates/persistence/src/membership/invitations.rs`
- Modify: the three other `NewInvitation { .. }` sites: `handover.rs:248`, `signup.rs:403` and `requests.rs:363`
- Modify: `backend/crates/persistence/src/membership/mod.rs` (exports)
- Modify: every `CreateAccessRequest { .. }` and `IssueInvitation { .. }` literal in `crates/app/tests/**` (6 and 7 sites; find them with `grep -rn "CreateAccessRequest {\|IssueInvitation {" crates/app/tests`), adding `message: None`
- Modify: `backend/crates/app/Cargo.toml` `[dev-dependencies]`: `fau-crypto = { path = "../crypto" }`
- Test: `backend/crates/app/tests/requests.rs`, `backend/crates/app/tests/invitations.rs`

**Interfaces:**
- Consumes: `fau_crypto::{Sealed, Ciphertext, Aad, seal, open, encrypt, decrypt, generate_sealing_pair, DataKey, KeyId}`.
- Produces:
  - `AccessRequestMessage { request_id: Uuid, sealed: Sealed }`, with `CreateAccessRequest.message: Option<AccessRequestMessage>`.
  - `access_request_message(pool, tenant_id, request_id, actor_membership_id, at: Moment) -> Result<Option<Sealed>, MembershipError>`. It requires an admin role valid today, checked before the row is read (the ordering rule in `membership/mod.rs`).
  - `InvitationMessage { invitation_id: Uuid, ciphertext: Ciphertext }`, with `IssueInvitation.message: Option<InvitationMessage>`.
  - `invitation_message(pool, token: &str, reader: &VerifiedEmail, at: Moment) -> Result<Option<InvitationMessageView>, MembershipError>`, with `InvitationMessageView { tenant_id: Uuid, invitation_id: Uuid, ciphertext: Ciphertext }`. It gives `UnknownInvitation` for a malformed, unknown, accepted, revoked or expired token, or for a reader who is not the recipient, and never says which.
  - Constants for the associated data, so writer and reader cannot drift:
    - `ACCESS_REQUEST_MESSAGE_AAD: (&str, &str) = ("access_requests", "sealed_message")`;
    - `INVITATION_MESSAGE_AAD: (&str, &str) = ("invitations", "encrypted_message")`.
  - `MESSAGE_MAX_BYTES: usize = 2200`, matching the check constraints in `0003`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/tests/requests.rs`, change the helper to carry no message:

```rust
fn access(fau: &Fau, requester: &str) -> CreateAccessRequest {
    CreateAccessRequest { tenant_id: fau.tenant_id, requester: verified(requester), message: None }
}
```

Replace the assertion that begins `let sealed_message: Option<Vec<u8>> =` in
`an_access_request_reaches_the_admins_and_names_nobody` with:

```rust
    let sealed_message: Option<Vec<u8>> =
        sqlx::query_scalar("select sealed_message from access_requests where id = $1")
            .bind(id).fetch_one(&pool).await.unwrap();
    assert!(sealed_message.is_none(), "no message given, none stored");
```

Append:

```rust
use fau_crypto::{generate_sealing_pair, open, seal, Aad};
use fau_persistence::membership::{access_request_message, AccessRequestMessage, ACCESS_REQUEST_MESSAGE_AAD};

#[tokio::test]
async fn a_sealed_message_is_stored_as_given_and_read_back_only_by_an_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let (pk, sk) = generate_sealing_pair();
    let request_id = Uuid::now_v7();
    let (table, column) = ACCESS_REQUEST_MESSAGE_AAD;
    let aad = Aad::new(fau.tenant_id, table, column, request_id);
    let sealed = seal(&pk, &aad, "Hei, jeg er ny forelder på 3. trinn").unwrap();

    let id = create_access_request(&pool, CreateAccessRequest {
        tenant_id: fau.tenant_id, requester: verified("ny@example.test"),
        message: Some(AccessRequestMessage { request_id, sealed: sealed.clone() }),
    }, t0).await.unwrap();
    assert_eq!(id, request_id, "the row takes the id the message is bound to");

    let stored = access_request_message(&pool, fau.tenant_id, id, fau.admin_membership_id, t0).await.unwrap().unwrap();
    assert_eq!(stored, sealed);
    assert_eq!(open(&sk, &aad, &stored).unwrap().as_str(), "Hei, jeg er ny forelder på 3. trinn");

    let plain_anywhere: i64 = sqlx::query_scalar(
        "select count(*) from access_requests where encode(sealed_message, 'escape') like '%forelder%'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(plain_anywhere, 0, "only ciphertext reaches the database");
}

#[tokio::test]
async fn a_non_admin_cannot_read_the_message_whether_or_not_the_request_exists() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let member = add_member(&pool, &fau, "m@example.test", new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)), t0).await;
    let id = create_access_request(&pool, access(&fau, "ny@example.test"), t0).await.unwrap();
    for request in [id, Uuid::now_v7()] {
        assert!(matches!(
            access_request_message(&pool, fau.tenant_id, request, member.membership_id, t0).await,
            Err(MembershipError::NotAuthorized)
        ));
    }
}

#[tokio::test]
async fn an_oversized_sealed_message_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let r = create_access_request(&pool, CreateAccessRequest {
        tenant_id: fau.tenant_id, requester: verified("ny@example.test"),
        message: Some(AccessRequestMessage { request_id: Uuid::now_v7(), sealed: fau_crypto::Sealed::from_stored(vec![0; 2201]) }),
    }, t0).await;
    assert!(matches!(r, Err(MembershipError::MessageTooLong)));
}
```

(`Accepted` must expose `membership_id`. If its field has another name, use that. Check
`pub struct Accepted` in `invitations.rs`.)

In `crates/app/tests/invitations.rs`, add `message: None` to every `IssueInvitation` literal and
append:

```rust
use fau_crypto::{decrypt, encrypt, Aad, DataKey, KeyId};
use fau_persistence::membership::{invitation_message, InvitationMessage, INVITATION_MESSAGE_AAD};

async fn invite_with_message(pool: &sqlx::PgPool, fau: &Fau, to: &str, text: &str, t: fau_domain::time::Moment)
    -> (fau_persistence::membership::IssuedInvitation, DataKey, Aad)
{
    let invitation_id = Uuid::now_v7();
    let key = DataKey::from_bytes([5u8; 32]);
    let (table, column) = INVITATION_MESSAGE_AAD;
    let aad = Aad::new(fau.tenant_id, table, column, invitation_id);
    let ct = encrypt(KeyId::new(Uuid::now_v7()), &key, &aad, text).unwrap();
    let issued = issue_invitation(pool, IssueInvitation {
        tenant_id: fau.tenant_id, actor_membership_id: fau.admin_membership_id, recipient: email(to),
        roles: vec![OfferedRole { role: new_role("Medlem", CapabilityClass::Member), period: period(day(2026, 9, 1), day(2027, 9, 1)) }],
        handover_grant_id: None,
        message: Some(InvitationMessage { invitation_id, ciphertext: ct }),
    }, t).await.unwrap();
    assert_eq!(issued.invitation_id, invitation_id);
    (issued, key, aad)
}

#[tokio::test]
async fn the_invitee_reads_the_message_before_accepting_and_nobody_else_can() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let (issued, key, aad) = invite_with_message(&pool, &fau, "ny@example.test", "Hei! Vi trenger deg som kasserer.", t0).await;
    let token = issued.token.expose().to_owned();

    let view = invitation_message(&pool, &token, &verified("ny@example.test"), t0).await.unwrap().unwrap();
    assert_eq!((view.tenant_id, view.invitation_id), (fau.tenant_id, issued.invitation_id));
    assert_eq!(decrypt(&key, &aad, &view.ciphertext).unwrap().as_str(), "Hei! Vi trenger deg som kasserer.");

    for (tok, reader) in [(token.as_str(), "annen@example.test"), (&"0".repeat(64)[..], "ny@example.test"), ("short", "ny@example.test")] {
        assert!(matches!(invitation_message(&pool, tok, &verified(reader), t0).await, Err(MembershipError::UnknownInvitation)), "{reader}");
    }
}

#[tokio::test]
async fn the_message_is_not_readable_once_the_invitation_is_settled_or_expired() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let (issued, _, _) = invite_with_message(&pool, &fau, "ny@example.test", "x", t0).await;
    let token = issued.token.expose().to_owned();
    let later = at("2027-01-01T00:00:00Z");
    assert!(matches!(invitation_message(&pool, &token, &verified("ny@example.test"), later).await, Err(MembershipError::UnknownInvitation)), "expired");

    let (issued, _, _) = invite_with_message(&pool, &fau, "to@example.test", "y", t0).await;
    let token = issued.token.expose().to_owned();
    accept_invitation(&pool, AcceptInvitation { token: token.clone(), acceptor: verified("to@example.test"), admin_end_override: None }, t0).await.unwrap();
    assert!(matches!(invitation_message(&pool, &token, &verified("to@example.test"), t0).await, Err(MembershipError::UnknownInvitation)), "accepted");
}
```

(Use the imports `invitations.rs` already has for `issue_invitation`, `IssueInvitation`,
`OfferedRole`, `accept_invitation` and `AcceptInvitation`. Add whichever are missing.)

- [ ] **Step 2: Run to verify they fail**

Run: `TEST_DATABASE_URL=... cargo test -p fau-app --test requests --test invitations`
Expected: compile errors (`message` field, `access_request_message` and `invitation_message`
missing).

- [ ] **Step 3: Implement in `requests.rs`**

Replace the doc comment and struct of `CreateAccessRequest`:

```rust
/// The message, when given, is sealed to the FAU's public key by the caller (ADR-003
/// decision 6) with `Aad::new(tenant_id, ACCESS_REQUEST_MESSAGE_AAD.0,
/// ACCESS_REQUEST_MESSAGE_AAD.1, message.request_id)`. Persistence never sees plaintext.
#[derive(Clone)]
pub struct CreateAccessRequest {
    pub tenant_id: Uuid,
    pub requester: VerifiedEmail,
    pub message: Option<AccessRequestMessage>,
}

/// A sealed message and the id of the request row it is bound to.
#[derive(Debug, Clone)]
pub struct AccessRequestMessage {
    pub request_id: Uuid,
    pub sealed: fau_crypto::Sealed,
}

pub const ACCESS_REQUEST_MESSAGE_AAD: (&str, &str) = ("access_requests", "sealed_message");
/// The `*_is_bounded` check constraints in migration 0003.
pub const MESSAGE_MAX_BYTES: usize = 2200;
```

Extend the hand-written `Debug` with `.field("message", &self.message.as_ref().map(|_| "[sealed]"))`.

Extend `NewRequest` with two fields, `id: Option<Uuid>` and `sealed_message: Option<&'a [u8]>`.
In `insert_request`, use `let id = r.id.unwrap_or_else(Uuid::now_v7);`, and add `sealed_message`
to the column list and a `$12` bind:

```rust
        "insert into access_requests
           (tenant_id, id, kind, requester_email, invitee_email, requester_membership_id,
            replaced_assignment_id, proposed_starts_on, proposed_ends_on_exclusive,
            created_on, created_at, sealed_message)
         values ($1, $2, $3, $4, $5, $6, $7, $8::date, $9::date, $10::date, $11::timestamptz, $12)",
```

with `.bind(r.sealed_message)` last. In `create_access_request`, before `lock_tenant`:

```rust
    if req.message.as_ref().is_some_and(|m| m.sealed.as_bytes().len() > MESSAGE_MAX_BYTES) {
        return Err(MembershipError::MessageTooLong);
    }
```

and in the `NewRequest` literal:

```rust
            id: req.message.as_ref().map(|m| m.request_id),
            sealed_message: req.message.as_ref().map(|m| m.sealed.as_bytes()),
```

The replacement-proposal `NewRequest` literal gets `id: None, sealed_message: None`.

Append the reader:

```rust
/// The sealed message on a request, for the approval screen (flow spec §5.2). Authority
/// first: an admin valid today, checked before the request row is read.
pub async fn access_request_message(
    pool: &PgPool,
    tenant_id: Uuid,
    request_id: Uuid,
    actor_membership_id: Uuid,
    at: Moment,
) -> Result<Option<fau_crypto::Sealed>, MembershipError> {
    let mut tx = pool.begin().await?;
    require_admin(&mut tx, tenant_id, actor_membership_id, at.today()).await?;
    let row: Option<Option<Vec<u8>>> = sqlx::query_scalar(
        "select sealed_message from access_requests where tenant_id = $1 and id = $2")
        .bind(tenant_id).bind(request_id).fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    Ok(row.flatten().map(fau_crypto::Sealed::from_stored))
}
```

- [ ] **Step 4: Implement in `invitations.rs`**

`IssueInvitation` gains `pub message: Option<InvitationMessage>`. Add:

```rust
/// A message encrypted under the invitation's key (ADR-003 decision 6) with
/// `Aad::new(tenant_id, INVITATION_MESSAGE_AAD.0, INVITATION_MESSAGE_AAD.1, invitation_id)`,
/// and the id of the invitation row it is bound to.
#[derive(Debug, Clone)]
pub struct InvitationMessage {
    pub invitation_id: Uuid,
    pub ciphertext: fau_crypto::Ciphertext,
}

pub const INVITATION_MESSAGE_AAD: (&str, &str) = ("invitations", "encrypted_message");

#[derive(Debug, Clone)]
pub struct InvitationMessageView {
    pub tenant_id: Uuid,
    pub invitation_id: Uuid,
    pub ciphertext: fau_crypto::Ciphertext,
}
```

`NewInvitation` gains `pub(crate) id: Option<Uuid>` and `pub(crate) encrypted_message: Option<Vec<u8>>`.
In `insert_invitation`, use `let invitation_id = new.id.unwrap_or_else(Uuid::now_v7);`, and add
`encrypted_message` as column/`$12` with `.bind(new.encrypted_message)`. In `issue_invitation`,
check `MESSAGE_MAX_BYTES` first (import it from `super::requests`) and pass:

```rust
            id: req.message.as_ref().map(|m| m.invitation_id),
            encrypted_message: req.message.as_ref().map(|m| m.ciphertext.as_bytes().to_vec()),
```

The other three `NewInvitation` literals get `id: None, encrypted_message: None`. `resend_invitation`
updates only `token_hash` and `expires_at`, so a resend keeps the message, as decided on
24 September. There is no code change for that, but add this assertion to the second new test:
after `resend_invitation`, `invitation_message` with the **new** token still returns the
ciphertext.

Append the reader:

```rust
/// The invitee's view of the message (decision of 24 September): a valid, unused,
/// unexpired token, read by the verified recipient. Every other case is
/// `UnknownInvitation`, so the answer never says which part failed.
pub async fn invitation_message(
    pool: &PgPool,
    token: &str,
    reader: &VerifiedEmail,
    at: Moment,
) -> Result<Option<InvitationMessageView>, MembershipError> {
    if !looks_like_token(token) {
        return Err(MembershipError::UnknownInvitation);
    }
    let row: Option<(Uuid, Uuid, String, Option<Vec<u8>>)> = sqlx::query_as(
        "select tenant_id, id, recipient_email, encrypted_message from invitations
          where token_hash = $1 and accepted_at is null and revoked_at is null
            and expires_at > $2::timestamptz")
        .bind(hash_token(token)).bind(ts_param(at.now())).fetch_optional(pool).await?;
    let (tenant_id, invitation_id, recipient, message) = row.ok_or(MembershipError::UnknownInvitation)?;
    if recipient != reader.email().as_str() {
        return Err(MembershipError::UnknownInvitation);
    }
    Ok(message.map(|m| InvitationMessageView { tenant_id, invitation_id, ciphertext: fau_crypto::Ciphertext::from_stored(m) }))
}
```

Export the new items from `membership/mod.rs`:
- `access_request_message`, `AccessRequestMessage`, `ACCESS_REQUEST_MESSAGE_AAD` and `MESSAGE_MAX_BYTES` from `requests`;
- `invitation_message`, `InvitationMessage`, `InvitationMessageView` and `INVITATION_MESSAGE_AAD` from `invitations`.

Remove the two doc lines "No message field until the key service exists" from `requests.rs`
(one on `CreateAccessRequest`, which Step 3 replaced, and one on `CreateReplacementProposal`).
Replacement proposals carry no message by the flow spec, so the second becomes: "Carries no
message (flow spec §5.3)."

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test -p fau-app --test requests --test invitations --test schema_review --test membership_schema`
Expected: every test passes, including the unchanged `*_is_bounded` schema tests and
`schema_review`'s forbidden-column test (no column was added).

- [ ] **Step 6: Run the whole workspace**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green. Fix any other `CreateAccessRequest` / `IssueInvitation` literal the compiler
names.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/persistence backend/crates/app backend/Cargo.lock
git commit -m "Accept sealed access-request messages and encrypted invitation messages (#3418, #3506)"
```

---

### Task 10: The chain end to end, and "content unavailable" at the API edge

**Files:**
- Modify: `backend/crates/domain/src/error_code.rs` (new variant and its test)
- Modify: `backend/crates/app/Cargo.toml`:
  - `[dependencies]`: `fau-key-client = { path = "../key-client" }`;
  - `[dev-dependencies]`: `fau-key-service = { path = "../key-service", features = ["test-support"] }` and `fau-key-protocol = { path = "../key-protocol" }`.
- Modify: `backend/crates/app/src/http/error.rs`
- Create: `backend/crates/app/tests/key_chain.rs`

**Interfaces:**
- Produces:
  - `ErrorCode::ContentUnavailable`, which serialises as `"content_unavailable"`.
  - `ApiError::content_unavailable()`, which is a 503.
  - `impl From<fau_key_client::KeyError> for ApiError`.

**The mapping and why.** The error contract (app-foundation design §11) sends codes, never prose,
because the client renders the sentence. The Bokmål source string for this code is
"Innholdet er midlertidig utilgjengelig. Vi jobber med saken." It goes into
`frontend/src/locales/nb-NO.json` when the frontend exists (#3439); record it in the variant's doc
comment until then.

| `KeyError` | `ApiError` | Log |
|---|---|---|
| `Sealed`, `Unavailable`, `CeilingReached`, `RateLimited` | `content_unavailable` (503) | `WARN` with the variant name |
| `NotFound` | `not_found` (404) | none |
| `Refused`, `Invalid` | `internal_error` (500) | `ERROR`: a programming error on our side |

- [ ] **Step 1: Write the failing tests**

In `error_code.rs` tests:

```rust
    #[test]
    fn content_unavailable_serialises_as_snake_case() {
        assert_eq!(serde_json::to_string(&ErrorCode::ContentUnavailable).unwrap(), "\"content_unavailable\"");
    }
```

In `crates/app/src/http/error.rs`, add a test module:

```rust
#[cfg(test)]
mod key_error_tests {
    use super::*;
    use fau_key_client::KeyError;

    #[test]
    fn a_sealed_key_service_is_content_unavailable_not_an_internal_error() {
        for (e, status) in [
            (KeyError::Sealed, StatusCode::SERVICE_UNAVAILABLE),
            (KeyError::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
            (KeyError::CeilingReached, StatusCode::SERVICE_UNAVAILABLE),
            (KeyError::RateLimited, StatusCode::SERVICE_UNAVAILABLE),
            (KeyError::NotFound, StatusCode::NOT_FOUND),
            (KeyError::Refused, StatusCode::INTERNAL_SERVER_ERROR),
            (KeyError::Invalid, StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            assert_eq!(ApiError::from(e).status, status, "{e:?}");
        }
        assert_eq!(ApiError::from(KeyError::Sealed).code, ErrorCode::ContentUnavailable);
    }
}
```

`crates/app/tests/key_chain.rs`:

```rust
//! The whole chain (spec §5.5): the real key service over mTLS, the key client and
//! cache, fau-crypto and PostgreSQL. It proves the #3418 messages are sealed and
//! encrypted end to end, readable only through the key service, and that destroying
//! the FAU's KEK makes them unreadable.

mod common;
use std::sync::Arc;
use std::time::Duration;

use common::membership::*;
use common::TestDb;
use fau_crypto::{decrypt, encrypt, open, seal, Aad, SealingPrivateKey};
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_key_client::{ClientConfig, KeyCache, KeyClient, KeyError, Who};
use fau_key_protocol::KeyClass;
use fau_key_service::api::Limits;
use fau_key_service::test_support::spawn;
use fau_persistence::membership::*;
use uuid::Uuid;

#[tokio::test]
async fn messages_are_sealed_encrypted_and_shredded_through_the_key_service() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let srv = spawn(Limits::default(), true).await;
    let client = Arc::new(KeyClient::new(ClientConfig {
        base_url: srv.base_url.clone(), ca_pem: srv.ca_pem.clone(), identity_pem: srv.client_identity_pem.clone(),
        instance_id: "app-0".into(), timeout: Duration::from_secs(5),
    }).unwrap());
    let cache = KeyCache::new(client.clone(), Arc::new(jiff::Timestamp::now));
    let admin = Who { account_id: fau.admin_account_id, session_id: Uuid::now_v7() };
    client.create_fau(fau.tenant_id, &admin).await.unwrap();

    // A stranger's access request: sealed with the public key, no session involved.
    let pk = client.public_key(fau.tenant_id).await.unwrap();
    let request_id = Uuid::now_v7();
    let (t, c) = ACCESS_REQUEST_MESSAGE_AAD;
    let req_aad = Aad::new(fau.tenant_id, t, c, request_id);
    create_access_request(&pool, CreateAccessRequest {
        tenant_id: fau.tenant_id, requester: verified("ny@example.test"),
        message: Some(AccessRequestMessage { request_id, sealed: seal(&pk, &req_aad, "Jeg vil bli med i FAU").unwrap() }),
    }, t0).await.unwrap();

    // The admin's approval screen: unwrap the sealing key in the admin's session, open.
    let sealed = access_request_message(&pool, fau.tenant_id, request_id, fau.admin_membership_id, t0).await.unwrap().unwrap();
    let (_, sk) = cache.get(&admin, fau.tenant_id, KeyClass::Sealing, None).await.unwrap();
    assert_eq!(open(&SealingPrivateKey::from_bytes(*sk.expose()), &req_aad, &sealed).unwrap().as_str(), "Jeg vil bli med i FAU");

    // An invitation message under its own invitation key.
    let invitation_id = Uuid::now_v7();
    let leased = client.create_key(fau.tenant_id, KeyClass::Invitation, &invitation_id.to_string(), &admin).await.unwrap();
    let (t, c) = INVITATION_MESSAGE_AAD;
    let inv_aad = Aad::new(fau.tenant_id, t, c, invitation_id);
    let issued = issue_invitation(&pool, IssueInvitation {
        tenant_id: fau.tenant_id, actor_membership_id: fau.admin_membership_id, recipient: email("to@example.test"),
        roles: vec![OfferedRole { role: new_role("Medlem", CapabilityClass::Member), period: period(day(2026, 9, 1), day(2027, 9, 1)) }],
        handover_grant_id: None,
        message: Some(InvitationMessage { invitation_id, ciphertext: encrypt(leased.key_id, &leased.key, &inv_aad, "Velkommen!").unwrap() }),
    }, t0).await.unwrap();
    client.release(leased.lease_id).await.unwrap();

    // The invitee, not yet a member, in their own session.
    let invitee = Who { account_id: Uuid::now_v7(), session_id: Uuid::now_v7() };
    let view = invitation_message(&pool, issued.token.expose(), &verified("to@example.test"), t0).await.unwrap().unwrap();
    let (key_id, key) = cache.get(&invitee, view.tenant_id, KeyClass::Invitation, Some(&view.invitation_id.to_string())).await.unwrap();
    assert_eq!(fau_crypto::key_id_of(&view.ciphertext).unwrap(), key_id, "the envelope names the key it needs");
    assert_eq!(decrypt(&key, &inv_aad, &view.ciphertext).unwrap().as_str(), "Velkommen!");

    // Crypto-shredding: with the KEK gone, nothing under it can be unwrapped.
    cache.release_session(admin.session_id).await;
    cache.release_session(invitee.session_id).await;
    client.destroy_fau(fau.tenant_id, &admin).await.unwrap();
    let fresh = Who { account_id: fau.admin_account_id, session_id: Uuid::now_v7() };
    assert_eq!(cache.get(&fresh, fau.tenant_id, KeyClass::Sealing, None).await.unwrap_err(), KeyError::NotFound);
    assert_eq!(cache.get(&fresh, fau.tenant_id, KeyClass::Invitation, Some(&invitation_id.to_string())).await.unwrap_err(), KeyError::NotFound);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-domain error_code && cargo test -p fau-app --lib key_error_tests && cargo test -p fau-app --test key_chain`
Expected: compile errors (`ContentUnavailable`, `From<KeyError>`).

- [ ] **Step 3: Implement**

In `error_code.rs`:

```rust
    /// Encrypted content cannot be read right now: the key service is sealed,
    /// unreachable or at its limits (docs/key-service-design.md §5.4). Navigation, login
    /// and authorization still work. Bokmål source string for the client catalogue
    /// (#3439): "Innholdet er midlertidig utilgjengelig. Vi jobber med saken."
    ContentUnavailable,
```

In `http/error.rs`, give the test access to `status` and `code` by making the fields
`pub(crate)`. Leave the constructor pattern unchanged, then add:

```rust
impl ApiError {
    pub fn content_unavailable() -> Self {
        Self::new(ErrorCode::ContentUnavailable, StatusCode::SERVICE_UNAVAILABLE)
    }
}

impl From<fau_key_client::KeyError> for ApiError {
    fn from(e: fau_key_client::KeyError) -> Self {
        use fau_key_client::KeyError::*;
        match e {
            Sealed | Unavailable | CeilingReached | RateLimited => {
                tracing::warn!(key_error = ?e, "encrypted content unavailable");
                Self::content_unavailable()
            }
            NotFound => Self::not_found(),
            Refused | Invalid => {
                tracing::error!(key_error = ?e, "key service refused a request the backend should not have sent");
                Self::internal_error()
            }
        }
    }
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --workspace` (with `TEST_DATABASE_URL` set)
Expected: green, including `key_chain`.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/domain backend/crates/app backend/Cargo.lock
git commit -m "Prove the key chain end to end and map a sealed key service to content_unavailable (#3506)"
```

---

### Task 11: Image, compose, Kubernetes manifests and the operations runbook

**Files:**
- Modify: `Dockerfile` (two new stages)
- Modify: `compose.yaml` (`keys-certs` and `keys` services, two volumes)
- Modify: `.gitignore` (`**/.dev-certs/`)
- Modify: `backend/crates/app/tests/image.rs` (an ignored `key_service_image_contract` test)
- Create: `gitops/apps/key-service/{namespace,certificates,statefulset,service,networkpolicy,prometheusrule,kustomization}.yaml`
- Create: `docs/key-service-operations.md`

- [ ] **Step 1: Pin the runtime base**

The key-service runtime is `gcr.io/distroless/cc-debian12:nonroot`: glibc, CA certificates, no
shell, uid 65532. Resolve its digest and pin it, as every base image here is pinned
(`dockerfile_pins_base_images_by_digest` enforces it):

```bash
docker buildx imagetools inspect gcr.io/distroless/cc-debian12:nonroot --format '{{json .Manifest.Digest}}'
```

Use the printed `sha256:…` in Step 2.

- [ ] **Step 2: Write the failing image test** (append to `tests/image.rs`)

```rust
const KEYS_IMAGE: &str = "fau/key-service:test";

#[test]
#[ignore = "builds the release key-service image; run with --ignored"]
fn key_service_image_contract() {
    let root = repo_root();
    docker_ok(&["build", "--target", "key-service", "-t", KEYS_IMAGE, root.to_str().unwrap()]);
    // No shell in the image.
    assert!(!docker(&["run", "--rm", "--entrypoint", "/bin/sh", KEYS_IMAGE, "-c", "true"]).status.success());
    // Non-root.
    let user = String::from_utf8(docker_ok(&["image", "inspect", "-f", "{{.Config.User}}", KEYS_IMAGE]).stdout).unwrap();
    assert!(!user.trim().is_empty() && user.trim() != "0" && user.trim() != "root", "user was {user:?}");
    // The release binary refuses dev mode.
    let out = docker(&["run", "--rm", "-e", "FAU_KEYS_DEV_ROOT_KEY=x", KEYS_IMAGE, "serve"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("FAU_KEYS_DEV_ROOT_KEY: not permitted"));
    // --version reads no configuration.
    assert!(String::from_utf8_lossy(&docker_ok(&["run", "--rm", KEYS_IMAGE, "--version"]).stdout).starts_with("fau-keys "));
}
```

Run: `cargo test -p fau-app --test image key_service_image_contract -- --ignored`
Expected: FAIL (no `key-service` target).

- [ ] **Step 3: Add the Dockerfile stages** (after the existing `runtime` stage, so the default
target stays the app)

```dockerfile
# ---- The key service (docs/key-service-design.md §6.1) ------------------------------
# A separate trust boundary, so a separate image: one binary, no shell, no backend code.
FROM rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e AS key-service-build
WORKDIR /src
COPY backend/Cargo.toml backend/Cargo.lock backend/rust-toolchain.toml ./
COPY backend/crates ./crates
COPY backend/migrations ./migrations
ARG KEYS_FEATURES=""
# No `dev` feature in the release target: FAU_KEYS_DEV_ROOT_KEY is refused (spec §6.4).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p fau-key-service --bin fau-keys ${KEYS_FEATURES:+--features ${KEYS_FEATURES}} \
 && cp /src/target/release/fau-keys /fau-keys

FROM gcr.io/distroless/cc-debian12:nonroot@<DIGEST FROM STEP 1> AS key-service
COPY --from=key-service-build /fau-keys /fau-keys
USER 65532:65532
EXPOSE 8443 9464
ENTRYPOINT ["/fau-keys"]
CMD ["serve"]
```

Replace `<DIGEST FROM STEP 1>` with the digest (the only value in this plan the executor fills
in, because it must be read live). Then make sure the final stage of the file is still the app's
`runtime`: move the two new stages **above** `FROM debian:bookworm-slim… AS runtime`, or
`docker build` without `--target` would build the key service. Compose's `keys` service builds
`--target key-service` with `KEYS_FEATURES=dev`.

Run the Step 2 test again. Expected: PASS.

- [ ] **Step 4: Add the compose services**

```yaml
  # One-shot: a throwaway dev CA and certificates for the keys service and the app.
  keys-certs:
    image: fau/key-service:dev
    command: ["dev-certs", "--out", "/certs", "--server-name", "keys", "--server-name", "localhost"]
    volumes:
      - keys-certs:/certs
    restart: "no"

  keys:
    build:
      context: .
      dockerfile: Dockerfile
      target: key-service
      args:
        KEYS_FEATURES: dev
    image: fau/key-service:dev
    command: ["serve"]
    environment:
      FAU_KEYS_STORE: /data/keys.db
      FAU_KEYS_API_BIND: 0.0.0.0:8443
      FAU_KEYS_METRICS_BIND: 0.0.0.0:9464
      FAU_KEYS_UNSEAL_BIND: 127.0.0.1:8444
      FAU_KEYS_TLS_CERT: /certs/server.pem
      FAU_KEYS_TLS_KEY: /certs/server-key.pem
      FAU_KEYS_CLIENT_CA: /certs/ca.pem
      # A passphrase in dev builds only; the release image refuses this variable.
      FAU_KEYS_DEV_ROOT_KEY: ${FAU_KEYS_DEV_ROOT_KEY:-local-dev-only}
      LOG_LEVEL: ${LOG_LEVEL:-info}
    volumes:
      - keys-data:/data
      - keys-certs:/certs:ro
    depends_on:
      keys-certs:
        condition: service_completed_successfully
    read_only: true
    restart: "no"
```

Add `keys-data:` and `keys-certs:` under the top-level `volumes:`. The app does not depend on
`keys` yet: nothing in the app calls it until #3417 or #3501 adds the first HTTP consumer, and
that card wires `KEY_SERVICE_*` configuration and `depends_on`. (The spec's §6.3 says the app
waits for `keys`; that happens with the first consumer, not in this card.)

The `keys-certs` service writes key files as the image's uid 65532 into a fresh named volume.
If the volume's root is not writable for that uid, add `user: "0:0"` to `keys-certs` only. It
runs `dev-certs` and exits.

Check it runs:

```bash
docker compose -f compose.yaml build keys
docker compose -f compose.yaml up -d keys
docker compose -f compose.yaml logs --tail=20 keys   # expect "DEV MODE: unsealed" and "fau-keys listening"
docker compose -f compose.yaml rm -sf keys keys-certs
```

Use `-f compose.yaml`, not `/workspace/docker-compose.yml`. The latter is the agent's own stack
and must never be touched.

- [ ] **Step 5: Write the Kubernetes manifests** (`gitops/apps/key-service/`, deployed by #3424)

`namespace.yaml`:

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: fau-keys
```

`certificates.yaml` sets up an internal CA separate from the ACME issuers (spec §5.2): a
self-signed bootstrap issuer, a CA certificate, a CA issuer, and the server and app-client
certificates.

```yaml
apiVersion: cert-manager.io/v1
kind: Issuer
metadata: { name: fau-keys-bootstrap, namespace: fau-keys }
spec: { selfSigned: {} }
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata: { name: fau-keys-ca, namespace: fau-keys }
spec:
  isCA: true
  commonName: fau-keys-internal-ca
  secretName: fau-keys-ca
  duration: 87600h
  privateKey: { algorithm: ECDSA, size: 256 }
  issuerRef: { name: fau-keys-bootstrap, kind: Issuer }
---
apiVersion: cert-manager.io/v1
kind: Issuer
metadata: { name: fau-keys-ca, namespace: fau-keys }
spec: { ca: { secretName: fau-keys-ca } }
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata: { name: fau-keys-server, namespace: fau-keys }
spec:
  secretName: fau-keys-server-tls
  commonName: fau-keys
  dnsNames: [fau-keys.fau-keys.svc, fau-keys.fau-keys.svc.cluster.local]
  duration: 2160h
  renewBefore: 360h
  privateKey: { algorithm: ECDSA, size: 256, rotationPolicy: Always }
  issuerRef: { name: fau-keys-ca, kind: Issuer }
# The app's client certificate is issued by the same CA into the app's namespace by #3424
# (cert-manager cannot write a Secret into another namespace; #3424 either runs a second CA
# Issuer there from a copied CA secret, or uses trust-manager). Common name: fau-app.
```

`statefulset.yaml`:

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata: { name: fau-keys, namespace: fau-keys }
spec:
  serviceName: fau-keys
  replicas: 1   # ADR-003: a single, rarely rescheduled workload. No PodDisruptionBudget (spec §6.2).
  selector: { matchLabels: { app: fau-keys } }
  template:
    metadata: { labels: { app: fau-keys } }
    spec:
      automountServiceAccountToken: false
      securityContext: { runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, fsGroup: 65532, seccompProfile: { type: RuntimeDefault } }
      containers:
        - name: fau-keys
          image: fau-key-service
          args: ["serve"]
          ports:
            - { name: api, containerPort: 8443 }
            - { name: metrics, containerPort: 9464 }
          env:
            - { name: FAU_KEYS_STORE, value: /data/keys.db }
            - { name: FAU_KEYS_TLS_CERT, value: /tls/tls.crt }
            - { name: FAU_KEYS_TLS_KEY, value: /tls/tls.key }
            - { name: FAU_KEYS_CLIENT_CA, value: /tls/ca.crt }
          securityContext: { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: [ALL] } }
          livenessProbe: { httpGet: { path: /metrics, port: metrics }, periodSeconds: 20 }
          # No readinessProbe on sealed: a sealed pod must stay in the endpoints so the
          # backend gets `503 sealed`, not a connection failure (spec §4.4).
          resources: { requests: { cpu: 20m, memory: 32Mi }, limits: { memory: 128Mi } }
          volumeMounts:
            - { name: data, mountPath: /data }
            - { name: tls, mountPath: /tls, readOnly: true }
      volumes:
        - name: tls
          secret: { secretName: fau-keys-server-tls }
  volumeClaimTemplates:
    - metadata: { name: data }
      spec:
        storageClassName: hcloud-volumes   # explicit: two default StorageClasses exist (#3488)
        accessModes: [ReadWriteOnce]
        resources: { requests: { storage: 1Gi } }
```

(The liveness probe uses the plain-HTTP metrics port because the API port requires a client
certificate. `/metrics` answers while sealed.)

`service.yaml`:

```yaml
apiVersion: v1
kind: Service
metadata: { name: fau-keys, namespace: fau-keys }
spec:
  selector: { app: fau-keys }
  ports:
    - { name: api, port: 8443, targetPort: api }
    - { name: metrics, port: 9464, targetPort: metrics }
```

`networkpolicy.yaml`:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: fau-keys, namespace: fau-keys }
spec:
  podSelector: { matchLabels: { app: fau-keys } }
  policyTypes: [Ingress, Egress]
  ingress:
    - from:
        - namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: fau-app } }
          podSelector: { matchLabels: { app: fau-app } }
      ports: [{ port: 8443 }]
    - from:
        - namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: monitoring } }
      ports: [{ port: 9464 }]
  egress:
    - to:
        - namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: kube-system } }
          podSelector: { matchLabels: { k8s-app: kube-dns } }
      ports: [{ port: 53, protocol: UDP }, { port: 53, protocol: TCP }]
```

The namespace and label names `fau-app` / `monitoring` are #3424's to confirm. The
NetworkPolicy's header comment says so.

`prometheusrule.yaml` holds the alert rules for #3442 (spec §6.5):

```yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata: { name: fau-keys, namespace: fau-keys }
spec:
  groups:
    - name: fau-keys
      rules:
        - alert: KeyServiceSealed
          expr: key_service_sealed == 1
          for: 2m
          labels: { severity: critical }
        - alert: KeyServiceUnsealFailed
          expr: increase(key_service_unseal_failures_total[10m]) > 0
          labels: { severity: critical }
        - alert: KeyServiceManyNewFauKeys
          expr: increase(key_service_new_fau_leases_total[1h]) > 50
          labels: { severity: critical }
        - alert: KeyServiceFauDestructionEnqueued
          expr: increase(key_service_fau_destructions_enqueued_total[15m]) > 0
          labels: { severity: critical }
        - alert: KeyServiceYoungEpochDestroyAttempt
          expr: increase(key_service_young_epoch_destroy_attempts_total[15m]) > 0
          labels: { severity: critical }
        - alert: KeyServiceCeilingReached
          expr: increase(key_service_ceiling_refusals_total[15m]) > 0
          labels: { severity: warning }
        - alert: KeyServiceRateLimited
          expr: increase(key_service_rate_limited_total[15m]) > 0
          labels: { severity: warning }
```

The alert names are what reach Signal. Per the internal-supplier record, a check name must carry
no member data; these carry none. The "50 new FAU keys an hour" threshold is a starting value
for #3442 to tune. The cancellation alert comes with #3507, where cancellation is built.

`kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: fau-keys
resources: [namespace.yaml, certificates.yaml, statefulset.yaml, service.yaml, networkpolicy.yaml, prometheusrule.yaml]
images:
  - name: fau-key-service   # #3424 sets newName (registry) and digest
```

Validate syntax without a cluster: `kubectl kustomize gitops/apps/key-service > /dev/null`.
Expected: exit 0. Do **not** apply. Deploying is #3424's, and any change to the cluster needs
Erik's go-ahead.

- [ ] **Step 6: Write `docs/key-service-operations.md`**

Sections, all in English, with the commands exactly as below:

1. **What it is:** one paragraph and a pointer to `docs/key-service-design.md`.
2. **First start (Erik, on his own terminal, never through an agent):**

   ```bash
   kubectl -n fau-keys exec -it fau-keys-0 -- /fau-keys init --store /data/keys.db
   ```

   Store the printed line in Proton Pass as the "Key service live root key" item (#3481). It is
   shown once. Then unseal (next section). `init` refuses a store that is already initialised.
3. **Unsealing after every start:**

   ```bash
   kubectl -n fau-keys port-forward pod/fau-keys-0 8444:8444 &
   fau-keys unseal --url http://127.0.0.1:8444
   ```

   The key is read without echo. A wrong key leaves the service sealed and raises
   `KeyServiceUnsealFailed`. The `fau-keys` binary for the laptop comes from
   `cargo build --release -p fau-key-service --bin fau-keys`. The unseal port listens on the
   pod's loopback only.
4. **What sealed means:**
   - login, authorization and navigation work;
   - every encrypted field answers `content_unavailable`;
   - access requests can still be submitted, because sealing uses only the public key;
   - `KeyServiceSealed` pages after 2 minutes.
5. **Until #3507: volume loss is total.** The key store has no backup by design. Losing the PVC
   loses every key and so all content. Acceptable only while no real FAU data exists; #3507 is
   the pilot gate.
6. **Alerts:** the table from spec §6.5 and what to do for each. For a destruction enqueued that
   nobody asked for: the 7-day window is the time to act; cancellation arrives with #3507.
7. **Local development:**
   - `docker compose -f compose.yaml up -d keys`;
   - dev builds derive the root key from `FAU_KEYS_DEV_ROOT_KEY` (default `local-dev-only`);
   - the release image refuses the variable.
8. **Rotating the root key:** not built (spec §4.7).

- [ ] **Step 7: Update `.gitignore`**, adding:

```
# fau-keys dev-certs output when run outside compose.
**/.dev-certs/
```

- [ ] **Step 8: Run everything**

```bash
cd /workspace/backend
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p fau-key-service --features dev,test-support --all-targets -- -D warnings
TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres cargo test --workspace
cargo test -p fau-key-service --features test-support
cargo test -p fau-app --test image -- --ignored --test-threads=1
kubectl kustomize ../gitops/apps/key-service > /dev/null
```

Expected: all green. The ignored image tests build real images, so they are slow.

- [ ] **Step 9: Commit**

```bash
git add Dockerfile compose.yaml .gitignore gitops docs/key-service-operations.md backend/crates/app/tests/image.rs
git commit -m "Package the key service: image, compose, manifests, alert rules and runbook (#3506)"
```

---

## After the last task

- Run `superpowers:requesting-code-review` over the branch, focused on the Global Constraints
  and spec §7.
- On the Favro card #3506: post the result with the test evidence, attach
  `docs/key-service-operations.md` (Erik reads Favro, not the repo), and pin a `👤 Needs you`
  item for the review. Do **not** move #3506 to Done: Done needs Erik's review.
- `/workspace/CLAUDE.md`: add one line that the key service exists, where its runbook is, and
  that `init` and `unseal` are Erik's alone. Say in the handover that CLAUDE.md is not committed.
