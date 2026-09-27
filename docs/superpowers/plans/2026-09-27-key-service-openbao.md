# Key Service with OpenBao: Implementation Plan (#3506)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make encrypted fields possible for every FAU feature, using OpenBao's transit engine for key custody and envelope encryption in the backend. The first consumers are the two #3418 messages.

**Architecture:** OpenBao holds one transit key per shreddable unit (an FAU's records, a document, a chat month, an FAU's messages).
- The backend obtains data keys from OpenBao and stores only their wrapped form in Postgres (`wrapped_keys`).
- It unwraps each once per session into an in-memory `KeyCache`, and encrypts locally with XChaCha20-Poly1305.
- The two messages use transit `encrypt` / `decrypt` directly.
- Deletion is a `bao` CLI script: soft delete now, hard delete after 7 days.

**Tech Stack:** Rust 1.98.1, `vaultrs` 0.8 (the Vault/OpenBao API client), `chacha20poly1305` 0.10, `zeroize` 1, sqlx 0.8/Postgres, OpenBao 2.7.0 (`openbao/openbao:2.7.0@sha256:71156a1c6623a5fa3f5e61b0c6a8ead0faf0df29a778339188443551995d1315`), its official Helm chart (`openbao` 0.29.6), and `bao` CLI scripts.

**Spec:** `docs/key-service-design.md` (27 September revision). Read it first; section numbers below refer to it. It supersedes `2026-09-27-key-service-hand-written-superseded.md`; do not execute that one.

## Global Constraints

- **Plaintext data keys, message plaintext, the unseal key and root or app tokens never appear**
  in a log line, an error message, a `Debug` output or a panic message. Types that hold one have a
  hand-written redacting `Debug`.
- **Transit key names:**
  - `fau-<tenant>-record`, `fau-<tenant>-doc-<document>`, `fau-<tenant>-chat-<YYYY-MM>` and
    `fau-<tenant>-messages`, where tenant and document are lowercase hyphenated UUIDs;
  - every key is `aes256-gcm96`, created with `exportable=false` and
    `allow_plaintext_backup=false`;
  - `deletion_allowed` is only ever set by the deletion script (§3.1).
- **Policies use the one-segment wildcard `+`, never `*`.**
  - `transit/keys/fau-*` would also match `…/config`, `…/trim`, `…/rotate` and `…/soft-delete`.
    The spike on 27 September showed `update` on a `*` path lets the holder set `deletion_allowed`
    and trim.
  - The policy files in Task 2 are exact. Change them only with the policy test (Task 3).
- **The app can:**
  - create keys (`update` on `transit/keys/+`);
  - generate data keys, encrypt and decrypt.

  **The app cannot:** read, list, configure, rotate, trim, export, back up, soft-delete, restore
  or delete keys.
- **The operator can:** list keys, read key metadata (to confirm `soft_deleted`), set config,
  soft-delete, restore and delete, and read and write the queue mount `fau-keys-queue/`. **The operator cannot:** encrypt, decrypt or generate data
  keys.
- **Envelope:** `version (1 byte = 1) ‖ nonce (24) ‖ XChaCha20-Poly1305 ciphertext+tag`.
- **Associated data:** `"fau-aad-v1" ‖ tenant (16) ‖ len‖table ‖ len‖column ‖ row id (16)` (§3.2).
- **Chat months** are Europe/Oslo calendar months. The deletion script refuses to delete a month
  that ended less than 12 months ago (§5).
- **Seven days** between soft delete and hard delete (ADR-003 decision 7).
- **The API edge maps sealed or unreachable OpenBao to `content_unavailable` (503).** Bokmål
  catalogue string: "Innholdet er midlertidig utilgjengelig. Vi jobber med saken." (§6).
- **`bao operator init`, `unseal` and `generate-root` against a real (non-dev) OpenBao are Erik's
  alone, on his own terminal.** The agent runs only the dev server in compose, with a dev root
  token (§4.2).
- **Compose** means `docker compose -f compose.yaml` (the `fau-app` project), never
  `/workspace/docker-compose.yml`, which is the agent's own stack.
- **Test commands** run from `/workspace/backend` with
  `TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres`,
  `TEST_OPENBAO_ADDR=http://openbao:8200` and `TEST_OPENBAO_TOKEN=dev-only-root`. The agent
  container is already attached to `fau-app_default`, so both hostnames resolve. If it is not,
  follow the memory note "agent reaches app DB" and warn Erik first, because attaching reloads the
  terminal.
- **Commits:** end every message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  If git has no identity, pass `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as`
  and the same `GIT_COMMITTER_*` per command. Never change git config and never push.
- **English** throughout. The only user-facing string is the Bokmål one above.

## Verified on 27 September 2026 against OpenBao 2.7.0 (spike, throwaway)

- `create` does not apply to `transit/keys/:name`; creating a key needs `update`.
- With a `+` policy, the app **cannot** use `config`, `rotate`, `trim`, `soft-delete`,
  `soft-delete-restore`, `export`, `backup`, `read` or `list`, and **can** create, `datakey`,
  `encrypt` and `decrypt`.
- The operator policy cannot `decrypt` or `datakey`.
- Transit `encrypt` with the app policy does **not** auto-create a missing key: permission denied.
  (With a root token it does, so tests that use root must create keys explicitly.)
- A soft-deleted key refuses `decrypt` and `datakey` ("refusing to use soft-deleted key"). The
  app's create call on a soft-deleted key returns its metadata **without restoring it**.
- Hard delete needs `deletion_allowed=true` first. After a hard delete, a re-created key of the
  same name does **not** open old wrapped keys.
- `associated_data` binds `encrypt` / `decrypt`; a mismatch gives 400
  "cipher: message authentication failed".
- A missing key gives 400 "encryption key not found". Sealed gives HTTP 503
  `{"errors":["Vault is sealed"]}`.
- Transit key metadata carries **no soft-delete timestamp**, hence the operator queue mount in
  Task 8.
- `vault_core_unsealed` exists in `/v1/sys/metrics?format=prometheus`.
- `vaultrs` 0.8 has `transit::key::create`, `transit::generate::data_key`
  (`DataKeyType::{Plaintext, Wrapped}`), and `transit::data::{encrypt, decrypt}` with
  `associated_data` on both builders. It sends `X-Vault-Token`, which OpenBao accepts.
- **Helm is not installed in the agent container.** Task 9 writes the chart values, but renders
  them only where `helm` exists, or leaves that to #3424. Adding `helm` to `Dockerfile.agent` is a
  separate change needing an image rebuild.

## Rulings this plan makes (for Erik's review)

1. **The 7-day queue lives in an operator-only OpenBao KV mount, `fau-keys-queue/`.** OpenBao
   records no soft-delete time. Keeping the queue beside the keys keeps it out of the app's reach
   and in the store #3507 replicates.
2. **Message columns widen from 2,200 to 4,096 bytes.** Transit ciphertext is
   `vault:v1:` + base64, about 2,713 bytes for a 500-character message. Migration 0003 is applied to
   the dev database, so migration `0005` alters the two check constraints; 0003 is not edited.
3. **No key id inside the local envelope.** One `wrapped_keys` row per unit means the unit is
   always known from context. The version byte leaves room to add one if keys are ever rotated.
4. **Only a failed authentication (403) triggers the client's one re-login-and-retry.** That covers
   an expired Kubernetes auth token. Any other error is returned at once.

## File Structure

```
backend/
  Cargo.toml                                   modify: members += crypto, keys; workspace deps
  migrations/0005_wrapped_keys.sql             NEW  wrapped_keys; widen message check constraints
  crates/
    crypto/                                    NEW  fau-crypto
      src/{lib,key,envelope,error}.rs          DataKey, Aad, Ciphertext, encrypt, decrypt
    keys/                                      NEW  fau-keys (OpenBao client + session cache)
      src/lib.rs                               re-exports
      src/unit.rs                              Unit -> transit key name
      src/client.rs                            Keys: create key, data key, unwrap, messages; KeyError
      src/cache.rs                             KeyCache (ADR-003 5a)
      tests/openbao.rs                         client + policy matrix against TEST_OPENBAO_ADDR
      tests/cache.rs                           cache semantics against TEST_OPENBAO_ADDR
    persistence/src/keys.rs                    NEW  wrapped-key get-or-create inside a transaction
    persistence/src/membership/{requests,invitations,handover,signup,mod}.rs   modify: messages
    domain/src/error_code.rs                   modify: ContentUnavailable
    app/src/http/error.rs                      modify: From<KeyError>
    app/tests/{requests,invitations,schema_review}.rs   modify
    app/tests/key_chain.rs                     NEW  end to end: Postgres + OpenBao
ops/openbao/                                   kustomize root for the cluster; mounted at /ops in compose
  policies/fau-app.hcl                         NEW
  policies/fau-keys-operator.hcl               NEW
  configure.sh                                 NEW  transit, queue mount, audit, policies, auth, quotas
  shred.sh                                     NEW  soft-delete / restore / finalize / chat-expire
  test-shred.sh                                NEW  the script's own tests, run inside the openbao container
  job.sh                                       NEW  CronJob entrypoint: Kubernetes login, then shred.sh
  helm-values.yaml                             NEW  values for the official openbao chart
  kustomization.yaml, k8s/*.yaml               NEW  namespace, CA Issuer, NetworkPolicy, CronJobs, alerts
compose.yaml                                   modify: openbao (dev) + openbao-config
docs/key-service-operations.md                 NEW  runbook
```

---
### Task 1: `fau-crypto`: the local envelope, data keys and units

**Files:**
- Modify: `backend/Cargo.toml`
- Create: `backend/crates/crypto/Cargo.toml`, `src/{lib,key,envelope,unit,error}.rs`
- Test: unit tests in each module

**Interfaces:**
- Produces:
  - `DataKey`: `from_bytes([u8; 32])`, `expose() -> &[u8; 32]`, with a redacted `Debug`.
  - `Aad::new(tenant_id: Uuid, table: &'static str, column: &'static str, row_id: Uuid)`, `.to_bytes() -> Vec<u8>`.
  - `Ciphertext`: `from_stored(Vec<u8>)`, `as_bytes()`, `len()`, with a redacted `Debug`.
  - `encrypt(key: &DataKey, aad: &Aad, plaintext: &str) -> Result<Ciphertext, CryptoError>`.
  - `decrypt(key: &DataKey, aad: &Aad, ct: &Ciphertext) -> Result<Zeroizing<String>, CryptoError>`.
  - `CryptoError { Randomness, Malformed, Decrypt }`.
  - `ChatMonth { year: i16, month: i8 }`: `parse("YYYY-MM") -> Option<ChatMonth>`, `Display` (`YYYY-MM`), `index() -> i32` (`year*12 + month - 1`).
  - `Unit { Record { tenant: Uuid }, Document { tenant: Uuid, document: Uuid }, Chat { tenant: Uuid, month: ChatMonth }, Messages { tenant: Uuid } }`, with:
    - `.tenant()`;
    - `.key_name() -> String`, per the Global Constraints;
    - `.storage() -> Option<(&'static str, Option<String>)>`, giving `wrapped_keys`' `(unit, scope)` and `None` for `Messages`, which has no data key. The column is `scope`, not `subject`: `schema_review` allows a column named `subject` only in `identity_mappings` (ADR-003 decision 3).
  - `WrappedKey`: `new(String) -> Option<WrappedKey>` (accepts only a `vault:v<N>:` prefix), `as_str()`.
  - `MessageCiphertext`: `new(String) -> Option<MessageCiphertext>` (same prefix rule), `as_str()`.

`Unit`, `WrappedKey` and `MessageCiphertext` live here, not in `fau-keys`, so `fau-persistence`
can use them without depending on an HTTP client.

- [ ] **Step 1: Workspace**

In `backend/Cargo.toml`:
- set `members = ["crates/domain", "crates/persistence", "crates/app", "crates/register-sources", "crates/crypto", "crates/keys"]`;
- add to `[workspace.dependencies]`:

```toml
chacha20poly1305 = "0.10"
zeroize = "1"
base64 = "0.22"
vaultrs = "0.8"
```

Create `crates/keys` as a stub for now: a `Cargo.toml` with `name = "fau-keys"` and the
workspace-inherited `version` / `edition` / `license`, `publish = false`, plus an empty
`src/lib.rs`. Task 3 fills it in.

`crates/crypto/Cargo.toml`:

```toml
[package]
name = "fau-crypto"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

# Backend-side encryption types (docs/key-service-design.md §3.2). No I/O, no HTTP, no SQL.
[dependencies]
chacha20poly1305 = { workspace = true }
getrandom = { workspace = true }
uuid = { workspace = true }
zeroize = { workspace = true }
```

- [ ] **Step 2: Write the failing tests**

`src/envelope.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fixture() -> (DataKey, Aad) {
        (DataKey::from_bytes([9u8; 32]), Aad::new(Uuid::now_v7(), "groups", "encrypted_name", Uuid::now_v7()))
    }

    #[test]
    fn round_trips() {
        let (key, aad) = fixture();
        let ct = encrypt(&key, &aad, "Juleballkomiteen").unwrap();
        assert_eq!(decrypt(&key, &aad, &ct).unwrap().as_str(), "Juleballkomiteen");
    }

    #[test]
    fn a_value_moved_to_another_row_column_table_or_fau_does_not_decrypt() {
        let (key, aad) = fixture();
        let ct = encrypt(&key, &aad, "x").unwrap();
        for wrong in [
            Aad { row_id: Uuid::now_v7(), ..aad.clone() },
            Aad { column: "encrypted_title", ..aad.clone() },
            Aad { table: "events", ..aad.clone() },
            Aad { tenant_id: Uuid::now_v7(), ..aad.clone() },
        ] {
            assert_eq!(decrypt(&key, &wrong, &ct).unwrap_err(), CryptoError::Decrypt);
        }
    }

    #[test]
    fn the_associated_data_is_length_prefixed() {
        let t = Uuid::nil();
        let r = Uuid::nil();
        assert_ne!(Aad::new(t, "ab", "c", r).to_bytes(), Aad::new(t, "a", "bc", r).to_bytes());
    }

    #[test]
    fn tampering_truncation_and_unknown_versions_are_refused() {
        let (key, aad) = fixture();
        let ct = encrypt(&key, &aad, "hei").unwrap();
        let mut b = ct.as_bytes().to_vec();
        *b.last_mut().unwrap() ^= 1;
        assert_eq!(decrypt(&key, &aad, &Ciphertext::from_stored(b)).unwrap_err(), CryptoError::Decrypt);
        assert_eq!(decrypt(&key, &aad, &Ciphertext::from_stored(vec![1, 2, 3])).unwrap_err(), CryptoError::Malformed);
        let mut v = ct.as_bytes().to_vec();
        v[0] = 2;
        assert_eq!(decrypt(&key, &aad, &Ciphertext::from_stored(v)).unwrap_err(), CryptoError::Malformed);
    }

    #[test]
    fn equal_plaintexts_encrypt_differently() {
        let (key, aad) = fixture();
        assert_ne!(encrypt(&key, &aad, "a").unwrap().as_bytes(), encrypt(&key, &aad, "a").unwrap().as_bytes());
    }

    #[test]
    fn debug_prints_no_bytes() {
        let (key, aad) = fixture();
        let ct = encrypt(&key, &aad, "hemmelig").unwrap();
        assert_eq!(format!("{ct:?}"), format!("Ciphertext({} bytes)", ct.len()));
        assert_eq!(format!("{key:?}"), "DataKey([redacted])");
    }
}
```

`src/unit.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn key_names_follow_the_spec() {
        let t = Uuid::parse_str("0192d3f4-0000-7000-8000-000000000001").unwrap();
        let d = Uuid::parse_str("0192d3f4-0000-7000-8000-0000000000aa").unwrap();
        assert_eq!(Unit::Record { tenant: t }.key_name(), "fau-0192d3f4-0000-7000-8000-000000000001-record");
        assert_eq!(Unit::Document { tenant: t, document: d }.key_name(), "fau-0192d3f4-0000-7000-8000-000000000001-doc-0192d3f4-0000-7000-8000-0000000000aa");
        assert_eq!(Unit::Chat { tenant: t, month: ChatMonth::parse("2026-09").unwrap() }.key_name(), "fau-0192d3f4-0000-7000-8000-000000000001-chat-2026-09");
        assert_eq!(Unit::Messages { tenant: t }.key_name(), "fau-0192d3f4-0000-7000-8000-000000000001-messages");
    }

    #[test]
    fn storage_rows_match_the_wrapped_keys_table() {
        let t = Uuid::now_v7();
        let d = Uuid::now_v7();
        assert_eq!(Unit::Record { tenant: t }.storage(), Some(("record", None)));
        assert_eq!(Unit::Document { tenant: t, document: d }.storage(), Some(("document", Some(d.to_string()))));
        assert_eq!(Unit::Chat { tenant: t, month: ChatMonth::parse("2026-09").unwrap() }.storage(), Some(("chat", Some("2026-09".into()))));
        assert_eq!(Unit::Messages { tenant: t }.storage(), None);
    }

    #[test]
    fn chat_months_parse_strictly() {
        assert_eq!(ChatMonth::parse("2026-09").unwrap().to_string(), "2026-09");
        for bad in ["2026-13", "2026-00", "26-09", "2026-9", "2026/09", ""] {
            assert!(ChatMonth::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(ChatMonth::parse("2027-10").unwrap().index() - ChatMonth::parse("2026-09").unwrap().index(), 13);
    }

    #[test]
    fn transit_ciphertexts_must_carry_the_vault_prefix() {
        assert!(WrappedKey::new("vault:v1:abc".into()).is_some());
        assert!(WrappedKey::new("plain".into()).is_none());
        assert!(MessageCiphertext::new("vault:v12:abc".into()).is_some());
        assert!(MessageCiphertext::new("vault:x:abc".into()).is_none());
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p fau-crypto`
Expected: compile errors.

- [ ] **Step 4: Implement**

`src/error.rs`:

```rust
use std::fmt;

/// Coarse on purpose: a caller learns that decryption failed, never why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError { Randomness, Malformed, Decrypt }

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
use zeroize::Zeroizing;

/// A 256-bit data key from OpenBao's `datakey`. Zeroised on drop; never printed.
#[derive(Clone)]
pub struct DataKey(Zeroizing<[u8; 32]>);

impl DataKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
    /// Named so every read of key bytes is greppable.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataKey([redacted])")
    }
}
```

`src/envelope.rs`:

```rust
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
        Self { tenant_id, table, column, row_id }
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
    pub fn from_stored(bytes: Vec<u8>) -> Self { Self(bytes) }
    pub fn as_bytes(&self) -> &[u8] { &self.0 }
    pub fn len(&self) -> usize { self.0.len() }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
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
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext.as_bytes(), aad: &aad.to_bytes() })
        .map_err(|_| CryptoError::Decrypt)?;
    let mut out = Vec::with_capacity(HEADER + body.len());
    out.push(VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&body);
    Ok(Ciphertext(out))
}

pub fn decrypt(key: &DataKey, aad: &Aad, ct: &Ciphertext) -> Result<Zeroizing<String>, CryptoError> {
    let b = &ct.0;
    if b.len() < HEADER + 16 || b[0] != VERSION {
        return Err(CryptoError::Malformed);
    }
    let plain = Zeroizing::new(
        XChaCha20Poly1305::new(Key::from_slice(key.expose()))
            .decrypt(XNonce::from_slice(&b[1..HEADER]), Payload { msg: &b[HEADER..], aad: &aad.to_bytes() })
            .map_err(|_| CryptoError::Decrypt)?,
    );
    let text = std::str::from_utf8(&plain).map_err(|_| CryptoError::Malformed)?;
    Ok(Zeroizing::new(text.to_owned()))
}
```

`src/unit.rs` (tests from Step 2 at the bottom):

```rust
//! The units that each have their own OpenBao transit key (docs/key-service-design.md §3.1).

use std::fmt;

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChatMonth { pub year: i16, pub month: i8 }

impl ChatMonth {
    pub fn parse(s: &str) -> Option<Self> {
        let (y, m) = s.split_once('-')?;
        if y.len() != 4 || m.len() != 2 || !y.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let (year, month) = (y.parse().ok()?, m.parse().ok()?);
        (1..=12).contains(&month).then_some(Self { year, month })
    }

    pub fn index(self) -> i32 {
        i32::from(self.year) * 12 + i32::from(self.month) - 1
    }
}

impl fmt::Display for ChatMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Record { tenant: Uuid },
    Document { tenant: Uuid, document: Uuid },
    Chat { tenant: Uuid, month: ChatMonth },
    Messages { tenant: Uuid },
}

impl Unit {
    pub fn tenant(&self) -> Uuid {
        match *self {
            Unit::Record { tenant } | Unit::Document { tenant, .. } | Unit::Chat { tenant, .. } | Unit::Messages { tenant } => tenant,
        }
    }

    pub fn key_name(&self) -> String {
        match self {
            Unit::Record { tenant } => format!("fau-{tenant}-record"),
            Unit::Document { tenant, document } => format!("fau-{tenant}-doc-{document}"),
            Unit::Chat { tenant, month } => format!("fau-{tenant}-chat-{month}"),
            Unit::Messages { tenant } => format!("fau-{tenant}-messages"),
        }
    }

    /// `(unit, scope)` in `wrapped_keys`; `None` for units without a data key.
    pub fn storage(&self) -> Option<(&'static str, Option<String>)> {
        match self {
            Unit::Record { .. } => Some(("record", None)),
            Unit::Document { document, .. } => Some(("document", Some(document.to_string()))),
            Unit::Chat { month, .. } => Some(("chat", Some(month.to_string()))),
            Unit::Messages { .. } => None,
        }
    }
}

fn has_transit_prefix(s: &str) -> bool {
    s.strip_prefix("vault:v")
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(v, body)| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && !body.is_empty())
}

/// A data key as OpenBao wrapped it (`vault:v1:…`). Useless once its transit key is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKey(String);

impl WrappedKey {
    pub fn new(s: String) -> Option<Self> { has_transit_prefix(&s).then_some(Self(s)) }
    pub fn as_str(&self) -> &str { &self.0 }
}

/// A message encrypted by OpenBao transit `encrypt` (`vault:v1:…`).
#[derive(Clone, PartialEq, Eq)]
pub struct MessageCiphertext(String);

impl MessageCiphertext {
    pub fn new(s: String) -> Option<Self> { has_transit_prefix(&s).then_some(Self(s)) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl fmt::Debug for MessageCiphertext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MessageCiphertext({} bytes)", self.0.len())
    }
}
```

`src/lib.rs`:

```rust
//! Backend-side encryption types for FAU (docs/key-service-design.md §3).

mod envelope;
mod error;
mod key;
mod unit;

pub use envelope::{decrypt, encrypt, Aad, Ciphertext};
pub use error::CryptoError;
pub use key::DataKey;
pub use unit::{ChatMonth, MessageCiphertext, Unit, WrappedKey};
pub use zeroize::Zeroizing;
```

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test -p fau-crypto`
Expected: 10 passed.

- [ ] **Step 6: Commit**

```bash
git add backend/Cargo.toml backend/Cargo.lock backend/crates/crypto backend/crates/keys
git commit -m "Add fau-crypto: the local envelope, data keys and key units (#3506)"
```

---

### Task 2: OpenBao for development: policies, configuration script, compose

**Files:**
- Create: `ops/openbao/policies/fau-app.hcl`, `ops/openbao/policies/fau-keys-operator.hcl`, `ops/openbao/configure.sh`
- Modify: `compose.yaml`

**Interfaces:**
- Produces:
  - a dev OpenBao at `http://openbao:8200`, root token `dev-only-root`, configured by the same `configure.sh` that production uses;
  - `configure.sh dev` also creates a periodic token `dev-only-app` under `fau-app` and `dev-only-operator` under `fau-keys-operator`. Tests in Tasks 3–8 use these names.

- [ ] **Step 1: Write the policies**

`ops/openbao/policies/fau-app.hcl`:

```hcl
# The FAU backend (docs/key-service-design.md §4.3). One path segment only (+): a `*` here
# would also grant transit/keys/<name>/config, /rotate, /trim and /soft-delete (verified
# 27 September 2026). Changing this file needs the policy matrix in crates/keys/tests.
path "transit/keys/+" { capabilities = ["update"] }         # create a key; never read, list or delete
path "transit/datakey/plaintext/+" { capabilities = ["update"] }
path "transit/datakey/wrapped/+" { capabilities = ["update"] }
path "transit/encrypt/+" { capabilities = ["update"] }
path "transit/decrypt/+" { capabilities = ["update"] }
```

`ops/openbao/policies/fau-keys-operator.hcl`:

```hcl
# The deletion job (docs/key-service-design.md §5). It can destroy keys but never read data:
# no encrypt, decrypt or datakey.
path "transit/keys" { capabilities = ["list"] }
path "transit/keys/+" { capabilities = ["read", "delete"] }   # read = metadata only (soft_deleted); export is a separate path
path "transit/keys/+/config" { capabilities = ["update"] }
path "transit/keys/+/soft-delete" { capabilities = ["delete"] }
path "transit/keys/+/soft-delete-restore" { capabilities = ["update"] }
# The 7-day queue: OpenBao keeps no soft-delete timestamp, so the job records its own.
path "fau-keys-queue" { capabilities = ["list"] }
path "fau-keys-queue/+" { capabilities = ["create", "read", "update", "delete"] }
```

- [ ] **Step 2: Write `ops/openbao/configure.sh`**

```sh
#!/bin/sh
# Configure OpenBao for FAU (docs/key-service-design.md §4.3). Idempotent: safe to re-run
# after any change to this file or the policies.
#
# Usage: BAO_ADDR=... BAO_TOKEN=<root or setup token> configure.sh dev|cluster
#   dev      compose and tests: token auth, fixed dev tokens (never outside compose)
#   cluster  Kubernetes auth roles for the app and the deletion job
set -eu
MODE="${1:?usage: configure.sh dev|cluster}"
HERE="$(cd "$(dirname "$0")" && pwd)"

has() { bao "$1" list -format=json 2>/dev/null | grep -q "\"$2/\""; }

has secrets transit || bao secrets enable transit
has secrets fau-keys-queue || bao secrets enable -path=fau-keys-queue -version=1 kv
# HMACs every value; key material and plaintext never reach the log (§4.4).
has audit stdout || bao audit enable -path=stdout file file_path=stdout

bao policy write fau-app "$HERE/policies/fau-app.hcl"
bao policy write fau-keys-operator "$HERE/policies/fau-keys-operator.hcl"

# Replaces the hand-written design's lease ceiling (§4.3); tuned under #3442.
bao write sys/quotas/rate-limit/fau-transit path=transit/ rate=50 interval=1s >/dev/null

case "$MODE" in
  dev)
    for t in "dev-only-app:fau-app" "dev-only-operator:fau-keys-operator"; do
      id="${t%%:*}"; policy="${t#*:}"
      bao token lookup "$id" >/dev/null 2>&1 || bao token create -id="$id" -policy="$policy" -period=768h -orphan >/dev/null
    done
    ;;
  cluster)
    has auth kubernetes || bao auth enable kubernetes
    bao write auth/kubernetes/config kubernetes_host="https://kubernetes.default.svc" >/dev/null
    bao write auth/kubernetes/role/fau-app \
      bound_service_account_names=fau-app bound_service_account_namespaces=fau-app \
      policies=fau-app token_ttl=1h token_max_ttl=24h >/dev/null
    bao write auth/kubernetes/role/fau-keys-operator \
      bound_service_account_names=fau-keys-operator bound_service_account_namespaces=openbao \
      policies=fau-keys-operator token_ttl=15m token_max_ttl=1h >/dev/null
    ;;
  *) echo "configure.sh: unknown mode $MODE" >&2; exit 2 ;;
esac
echo "configure.sh: done ($MODE)"
```

Make it executable: `chmod +x ops/openbao/configure.sh`.

- [ ] **Step 3: Add the compose services** (in `compose.yaml` under `services:`)

```yaml
  # OpenBao's dev server (docs/key-service-design.md §6): in-memory, auto-unsealed, fixed
  # dev root token. Development and tests only; production is the Helm chart (§4.1).
  openbao:
    image: openbao/openbao:2.7.0@sha256:71156a1c6623a5fa3f5e61b0c6a8ead0faf0df29a778339188443551995d1315
    command: ["server", "-dev", "-dev-root-token-id=dev-only-root", "-dev-listen-address=0.0.0.0:8200"]
    cap_add: [IPC_LOCK]
    volumes:
      - ${FAU_HOST_PATH:-.}/ops/openbao:/ops:ro
    healthcheck:
      test: ["CMD", "bao", "status", "-address=http://127.0.0.1:8200"]
      interval: 2s
      timeout: 2s
      retries: 30

  openbao-config:
    image: openbao/openbao:2.7.0@sha256:71156a1c6623a5fa3f5e61b0c6a8ead0faf0df29a778339188443551995d1315
    entrypoint: ["/bin/sh", "/ops/configure.sh", "dev"]
    environment:
      BAO_ADDR: http://openbao:8200
      BAO_TOKEN: dev-only-root
    volumes:
      - ${FAU_HOST_PATH:-.}/ops/openbao:/ops:ro
    depends_on:
      openbao:
        condition: service_healthy
    restart: "no"
```

The dev server keeps no state, so `openbao-config` must re-run after every `openbao` restart.

- [ ] **Step 4: Bring it up and verify the configuration** (this is the task's test)

```bash
cd /workspace
docker compose -f compose.yaml up -d openbao
docker compose -f compose.yaml run --rm openbao-config
docker compose -f compose.yaml exec -T -e BAO_ADDR=http://127.0.0.1:8200 -e BAO_TOKEN=dev-only-root openbao sh -c '
  bao secrets list | grep -E "^(transit|fau-keys-queue)/" &&
  bao audit list | grep -E "^stdout/" &&
  bao policy read fau-app | grep -F "transit/keys/+" &&
  bao token lookup dev-only-app | grep -E "policies.*fau-app"'
docker compose -f compose.yaml run --rm openbao-config   # a second run must also succeed
```

Expected: each `grep` finds its line, and both configuration runs print
`configure.sh: done (dev)`. If `bao status` is not the right healthcheck in this image, check
`docker compose -f compose.yaml exec openbao sh -c 'which bao; echo ok'`.

- [ ] **Step 5: Commit**

```bash
git add ops/openbao compose.yaml
git commit -m "Add OpenBao policies, the configuration script and a dev server in compose (#3506)"
```

---

### Task 3: `fau-keys`: the OpenBao client, and the policy matrix proven against a real OpenBao

**Files:**
- Replace the stub: `backend/crates/keys/Cargo.toml`, `src/lib.rs`
- Create: `backend/crates/keys/src/client.rs`
- Test: `backend/crates/keys/tests/openbao.rs`; a unit test for the error mapping in `client.rs`

**Interfaces:**
- Consumes: `fau_crypto::{Unit, DataKey, WrappedKey, MessageCiphertext, Aad, Zeroizing}`.
- Produces:
  - `Auth { Token(String), Kubernetes { role: String, jwt_path: PathBuf } }`, with a redacted `Debug`.
  - `KeysConfig { address: String, ca_cert_path: Option<String>, auth: Auth, timeout: Duration }`.
  - `Keys::connect(KeysConfig) -> Result<Keys, KeyError>`, which logs in for `Kubernetes`.
  - Methods:
    - `ensure_key(&Unit) -> Result<(), KeyError>`;
    - `new_data_key(&Unit) -> Result<(DataKey, WrappedKey), KeyError>`, which ensures the key first;
    - `unwrap(&Unit, &WrappedKey) -> Result<DataKey, KeyError>`;
    - `encrypt_message(tenant: Uuid, &Aad, &str) -> Result<MessageCiphertext, KeyError>`, which ensures `Messages` first;
    - `decrypt_message(tenant: Uuid, &Aad, &MessageCiphertext) -> Result<Zeroizing<String>, KeyError>`.
  - `KeyError { Sealed, NotFound, Forbidden, RateLimited, Invalid, Unavailable }`.
    - `NotFound` covers a missing key **or a soft-deleted one**: from the caller's side, both mean
      shredded or never created.
    - `Invalid` covers an authentication failure (wrong key or associated data) or a malformed
      request.

- [ ] **Step 1: Manifest**

```toml
[package]
name = "fau-keys"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

# The backend's use of OpenBao (docs/key-service-design.md §3, §6): a thin wrapper over
# vaultrs, the session cache, and the data-key flow over persistence. No crypto of its own.
[dependencies]
fau-crypto = { path = "../crypto" }
base64 = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
vaultrs = { workspace = true }
zeroize = { workspace = true }

[dev-dependencies]
reqwest = { version = "0.13", default-features = false, features = ["rustls", "json"] }
serde_json = { workspace = true }
```

- [ ] **Step 2: Write the failing tests** (`tests/openbao.rs`)

```rust
//! Against the dev OpenBao from Task 2 (TEST_OPENBAO_ADDR, TEST_OPENBAO_TOKEN = root).
//! Every test uses a fresh tenant id, so tests never collide and nothing needs cleaning.

use std::time::Duration;

use fau_crypto::{Aad, ChatMonth, Unit};
use fau_keys::{Auth, KeyError, Keys, KeysConfig};
use reqwest::StatusCode;
use uuid::Uuid;

fn addr() -> String { std::env::var("TEST_OPENBAO_ADDR").expect("TEST_OPENBAO_ADDR") }
fn root() -> String { std::env::var("TEST_OPENBAO_TOKEN").expect("TEST_OPENBAO_TOKEN") }

async fn app() -> Keys {
    Keys::connect(KeysConfig { address: addr(), ca_cert_path: None, auth: Auth::Token("dev-only-app".into()), timeout: Duration::from_secs(5) }).await.unwrap()
}

/// Raw calls for what the app must not do, or what only root/operator does.
async fn raw(token: &str, method: reqwest::Method, path: &str, body: Option<serde_json::Value>) -> StatusCode {
    let mut r = reqwest::Client::new().request(method, format!("{}/v1/{path}", addr())).header("X-Vault-Token", token);
    if let Some(b) = body { r = r.json(&b); }
    r.send().await.unwrap().status()
}

#[tokio::test]
async fn a_data_key_unwraps_to_the_same_bytes() {
    let keys = app().await;
    let unit = Unit::Record { tenant: Uuid::now_v7() };
    let (key, wrapped) = keys.new_data_key(&unit).await.unwrap();
    assert!(wrapped.as_str().starts_with("vault:v1:"));
    assert_eq!(keys.unwrap(&unit, &wrapped).await.unwrap().expose(), key.expose());
}

#[tokio::test]
async fn a_wrapped_key_opens_only_under_its_own_unit() {
    let keys = app().await;
    let tenant = Uuid::now_v7();
    let (_, wrapped) = keys.new_data_key(&Unit::Record { tenant }).await.unwrap();
    let other = Unit::Chat { tenant, month: ChatMonth::parse("2026-09").unwrap() };
    keys.ensure_key(&other).await.unwrap();
    assert_eq!(keys.unwrap(&other, &wrapped).await.unwrap_err(), KeyError::Invalid);
    assert_eq!(keys.unwrap(&Unit::Record { tenant: Uuid::now_v7() }, &wrapped).await.unwrap_err(), KeyError::NotFound);
}

#[tokio::test]
async fn messages_round_trip_and_are_bound_to_their_row() {
    let keys = app().await;
    let tenant = Uuid::now_v7();
    let aad = Aad::new(tenant, "access_requests", "encrypted_message", Uuid::now_v7());
    let ct = keys.encrypt_message(tenant, &aad, "Jeg vil bli med i FAU").await.unwrap();
    assert_eq!(keys.decrypt_message(tenant, &aad, &ct).await.unwrap().as_str(), "Jeg vil bli med i FAU");
    let moved = Aad { row_id: Uuid::now_v7(), ..aad };
    assert_eq!(keys.decrypt_message(tenant, &moved, &ct).await.unwrap_err(), KeyError::Invalid);
}

#[tokio::test]
async fn soft_delete_stops_use_restore_brings_it_back_and_hard_delete_is_final() {
    let keys = app().await;
    let unit = Unit::Record { tenant: Uuid::now_v7() };
    let (_, wrapped) = keys.new_data_key(&unit).await.unwrap();
    let name = unit.key_name();
    let root = root();
    assert!(raw(&root, reqwest::Method::DELETE, &format!("transit/keys/{name}/soft-delete"), None).await.is_success());
    assert_eq!(keys.unwrap(&unit, &wrapped).await.unwrap_err(), KeyError::NotFound);
    assert_eq!(keys.new_data_key(&unit).await.unwrap_err(), KeyError::NotFound, "creating does not resurrect a soft-deleted key");
    assert!(raw(&root, reqwest::Method::POST, &format!("transit/keys/{name}/soft-delete-restore"), None).await.is_success());
    assert!(keys.unwrap(&unit, &wrapped).await.is_ok());
    assert!(raw(&root, reqwest::Method::POST, &format!("transit/keys/{name}/config"), Some(serde_json::json!({"deletion_allowed": true}))).await.is_success());
    assert!(raw(&root, reqwest::Method::DELETE, &format!("transit/keys/{name}"), None).await.is_success());
    assert_eq!(keys.unwrap(&unit, &wrapped).await.unwrap_err(), KeyError::NotFound);
    keys.ensure_key(&unit).await.unwrap();
    assert_eq!(keys.unwrap(&unit, &wrapped).await.unwrap_err(), KeyError::Invalid, "a re-created key of the same name does not open old wrapped keys");
}

#[tokio::test]
async fn the_policies_allow_exactly_what_the_spec_says() {
    use reqwest::Method as M;
    let keys = app().await;
    let unit = Unit::Record { tenant: Uuid::now_v7() };
    let (_, wrapped) = keys.new_data_key(&unit).await.unwrap();
    let k = unit.key_name();
    let decrypt = Some(serde_json::json!({ "ciphertext": wrapped.as_str() }));
    let denied = |s: StatusCode| s == StatusCode::FORBIDDEN;

    // The app: everything but create / datakey / encrypt / decrypt is refused.
    let app_refused: Vec<(M, String, Option<serde_json::Value>)> = vec![
        (M::GET, format!("transit/keys/{k}"), None),
        (M::from_bytes(b"LIST").unwrap(), "transit/keys".into(), None),
        (M::DELETE, format!("transit/keys/{k}"), None),
        (M::POST, format!("transit/keys/{k}/config"), Some(serde_json::json!({"deletion_allowed": true}))),
        (M::POST, format!("transit/keys/{k}/rotate"), None),
        (M::POST, format!("transit/keys/{k}/trim"), Some(serde_json::json!({"min_available_version": 1}))),
        (M::DELETE, format!("transit/keys/{k}/soft-delete"), None),
        (M::POST, format!("transit/keys/{k}/soft-delete-restore"), None),
        (M::GET, format!("transit/export/encryption-key/{k}"), None),
        (M::GET, format!("transit/backup/{k}"), None),
        (M::POST, format!("transit/encrypt/fau-{}-messages", Uuid::now_v7()), Some(serde_json::json!({"plaintext": "aGVp"}))),
        (M::POST, "fau-keys-queue/x".into(), Some(serde_json::json!({"a": "b"}))),
    ];
    for (m, p, b) in app_refused {
        assert!(denied(raw("dev-only-app", m.clone(), &p, b).await), "app must not {m} {p}");
    }
    // The operator: can destroy, can never read data.
    for (m, p, b) in [
        (M::POST, format!("transit/decrypt/{k}"), decrypt.clone()),
        (M::POST, format!("transit/datakey/plaintext/{k}"), None),
        (M::POST, format!("transit/encrypt/{k}"), Some(serde_json::json!({"plaintext": "aGVp"}))),
    ] {
        assert!(denied(raw("dev-only-operator", m.clone(), &p, b).await), "operator must not {m} {p}");
    }
    assert!(raw("dev-only-operator", M::GET, &format!("transit/keys/{k}"), None).await.is_success(), "operator reads metadata");
    assert!(raw("dev-only-operator", M::from_bytes(b"LIST").unwrap(), "transit/keys", None).await.is_success());
}
```

Unit test in `src/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn api(code: u16, msg: &str) -> ClientError {
        ClientError::APIError { code, errors: vec![msg.to_owned()] }
    }

    #[test]
    fn openbao_errors_map_to_our_variants() {
        assert_eq!(map_err(api(503, "Vault is sealed")), KeyError::Sealed);
        assert_eq!(map_err(api(429, "request path \"transit/decrypt\": rate limit quota exceeded")), KeyError::RateLimited);
        assert_eq!(map_err(api(403, "permission denied")), KeyError::Forbidden);
        assert_eq!(map_err(api(400, "encryption key not found")), KeyError::NotFound);
        assert_eq!(map_err(api(400, "refusing to use soft-deleted key")), KeyError::NotFound);
        assert_eq!(map_err(api(400, "cipher: message authentication failed")), KeyError::Invalid);
    }

    #[test]
    fn a_token_is_never_printed() {
        assert!(!format!("{:?}", Auth::Token("s.SECRET".into())).contains("SECRET"));
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `TEST_OPENBAO_ADDR=http://openbao:8200 TEST_OPENBAO_TOKEN=dev-only-root cargo test -p fau-keys`
Expected: compile errors. The compose `openbao` must be up and configured (Task 2).

- [ ] **Step 4: Implement `src/client.rs`**

```rust
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
use vaultrs::api::transit::requests::{CreateKeyRequest, DataKeyType, DecryptDataRequest, EncryptDataRequest};
use vaultrs::client::{Client, VaultClient, VaultClientSettingsBuilder};
use vaultrs::error::ClientError;

const MOUNT: &str = "transit";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("OpenBao is sealed")] Sealed,
    #[error("no such key, or it has been shredded")] NotFound,
    #[error("OpenBao refused the request")] Forbidden,
    #[error("OpenBao's rate limit is reached")] RateLimited,
    #[error("the value could not be decrypted or the request was malformed")] Invalid,
    #[error("OpenBao is unreachable")] Unavailable,
}

pub(crate) fn map_err(e: ClientError) -> KeyError {
    match e {
        ClientError::APIError { code: 503, .. } => KeyError::Sealed,
        ClientError::APIError { code: 429, .. } => KeyError::RateLimited,
        ClientError::APIError { code: 403, .. } => KeyError::Forbidden,
        ClientError::APIError { code: 400, errors }
            if errors.iter().any(|m| m.contains("encryption key not found") || m.contains("soft-deleted")) => KeyError::NotFound,
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
            Auth::Kubernetes { role, jwt_path } => f.debug_struct("Kubernetes").field("role", role).field("jwt_path", jwt_path).finish(),
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
        let client = VaultClient::new(s.build().map_err(|_| KeyError::Invalid)?).map_err(|_| KeyError::Invalid)?;
        let keys = Self { client: RwLock::new(client), cfg };
        keys.login().await?;
        Ok(keys)
    }

    async fn login(&self) -> Result<(), KeyError> {
        if let Auth::Kubernetes { role, jwt_path } = &self.cfg.auth {
            let jwt = Zeroizing::new(tokio::fs::read_to_string(jwt_path).await.map_err(|_| KeyError::Unavailable)?);
            let mut c = self.client.write().await;
            let info = vaultrs::auth::kubernetes::login(&*c, "kubernetes", role, jwt.trim()).await.map_err(map_err)?;
            c.set_token(&info.client_token);
        }
        Ok(())
    }

    /// One retry after re-login on 403, for an expired Kubernetes auth token (plan ruling 4).
    async fn call<T>(&self, f: impl for<'a> Fn(&'a VaultClient) -> Call<'a, T>) -> Result<T, KeyError> {
        let first = { let c = self.client.read().await; f(&c).await };
        match first {
            Err(ClientError::APIError { code: 403, .. }) if matches!(self.cfg.auth, Auth::Kubernetes { .. }) => {
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
        let r = self.call(|c| {
            let name = name.clone();
            Box::pin(async move { vaultrs::transit::generate::data_key(c, MOUNT, &name, DataKeyType::Plaintext, None).await })
        })
        .await?;
        let plaintext = Zeroizing::new(r.plaintext.ok_or(KeyError::Invalid)?);
        Ok((decode32(&plaintext)?, WrappedKey::new(r.ciphertext).ok_or(KeyError::Invalid)?))
    }

    pub async fn unwrap(&self, unit: &Unit, wrapped: &WrappedKey) -> Result<DataKey, KeyError> {
        let (name, ct) = (unit.key_name(), wrapped.as_str().to_owned());
        let r = self.call(|c| {
            let (name, ct) = (name.clone(), ct.clone());
            Box::pin(async move { vaultrs::transit::data::decrypt(c, MOUNT, &name, &ct, None).await })
        })
        .await?;
        decode32(&Zeroizing::new(r.plaintext))
    }

    pub async fn encrypt_message(&self, tenant: Uuid, aad: &Aad, text: &str) -> Result<MessageCiphertext, KeyError> {
        let unit = Unit::Messages { tenant };
        self.ensure_key(&unit).await?;
        let (name, ad, pt) = (unit.key_name(), STANDARD.encode(aad.to_bytes()), Zeroizing::new(STANDARD.encode(text)));
        let r = self.call(|c| {
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

    pub async fn decrypt_message(&self, tenant: Uuid, aad: &Aad, ct: &MessageCiphertext) -> Result<Zeroizing<String>, KeyError> {
        let (name, ad, ct) = (Unit::Messages { tenant }.key_name(), STANDARD.encode(aad.to_bytes()), ct.as_str().to_owned());
        let r = self.call(|c| {
            let (name, ad, ct) = (name.clone(), ad.clone(), ct.clone());
            Box::pin(async move {
                let mut b = DecryptDataRequest::builder();
                b.associated_data(ad);
                vaultrs::transit::data::decrypt(c, MOUNT, &name, &ct, Some(&mut b)).await
            })
        })
        .await?;
        let bytes = Zeroizing::new(STANDARD.decode(Zeroizing::new(r.plaintext).as_bytes()).map_err(|_| KeyError::Invalid)?);
        Ok(Zeroizing::new(String::from_utf8(bytes.to_vec()).map_err(|_| KeyError::Invalid)?))
    }
}
```

`src/lib.rs`:

```rust
//! FAU's use of OpenBao (docs/key-service-design.md).

mod client;

pub use client::{Auth, KeyError, Keys, KeysConfig};
```

Notes for the implementer:
- `vaultrs`'s `ClientError::APIError { code, errors }`, the builder setters (`exportable`,
  `allow_plaintext_backup`, `associated_data`) and `Client::set_token` were read from vaultrs 0.8.0
  source on 27 September.
- If the `Call` higher-ranked closure fights the borrow checker, replace `call` with a
  `macro_rules!` that expands the same read-guard / retry logic inline. The behaviour must not
  change.
- A `LIST` request is `reqwest::Method::from_bytes(b"LIST")`, which OpenBao accepts. If not, use
  `GET` with `?list=true`.

- [ ] **Step 5: Run to verify they pass**

Run: `TEST_OPENBAO_ADDR=http://openbao:8200 TEST_OPENBAO_TOKEN=dev-only-root cargo test -p fau-keys`
Expected: 5 integration tests and 2 unit tests pass.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/keys backend/Cargo.lock
git commit -m "Add fau-keys: the OpenBao transit client, with the policy matrix proven (#3506)"
```

---

### Task 4: `wrapped_keys` and the wider message columns (migration 0005)

**Files:**
- Create: `backend/migrations/0005_wrapped_keys.sql`
- Create: `backend/crates/persistence/src/keys.rs`, exported from `lib.rs`
- Modify: `backend/crates/persistence/Cargo.toml` (`fau-crypto = { path = "../crypto" }`)
- Modify: `backend/crates/app/tests/schema_review.rs` (allowlist), `backend/crates/app/tests/membership_schema.rs` (renamed column and new bound)
- Test: `backend/crates/app/tests/wrapped_keys.rs` (new)

**Interfaces:**
- Produces:
  - `fau_persistence::keys::load_wrapped_key(conn: &mut PgConnection, unit: &Unit) -> Result<Option<WrappedKey>, WrappedKeyError>`.
  - `fau_persistence::keys::store_wrapped_key(conn: &mut PgConnection, unit: &Unit, key: &WrappedKey) -> Result<WrappedKey, WrappedKeyError>`, which returns **the stored one**: whichever of two racing sessions inserted first wins.
  - `WrappedKeyError { NoDataKey, Corrupt, Db(sqlx::Error) }`. `NoDataKey` is for `Unit::Messages`.
  - Schema:
    - `access_requests.sealed_message` becomes `encrypted_message` (the name was accurate for HPKE and is not now);
    - both message check constraints become `≤ 4096` bytes, named `access_request_encrypted_message_is_bounded` and `invitation_encrypted_message_is_bounded`.

- [ ] **Step 1: Write the failing tests** (`crates/app/tests/wrapped_keys.rs`)

```rust
//! `wrapped_keys` (docs/key-service-design.md §3.2): one wrapped data key per unit, the
//! first writer wins, and nothing but OpenBao's ciphertext is stored.

mod common;
use common::membership::*;
use common::TestDb;
use fau_crypto::{ChatMonth, Unit, WrappedKey};
use fau_persistence::keys::{load_wrapped_key, store_wrapped_key, WrappedKeyError};
use uuid::Uuid;

fn wk(s: &str) -> WrappedKey { WrappedKey::new(format!("vault:v1:{s}")).unwrap() }

#[tokio::test]
async fn the_first_stored_key_wins_and_is_loaded_back() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let mut conn = pool.acquire().await.unwrap();
    for unit in [
        Unit::Record { tenant: fau.tenant_id },
        Unit::Document { tenant: fau.tenant_id, document: Uuid::now_v7() },
        Unit::Chat { tenant: fau.tenant_id, month: ChatMonth::parse("2026-09").unwrap() },
    ] {
        assert_eq!(load_wrapped_key(&mut conn, &unit).await.unwrap(), None);
        assert_eq!(store_wrapped_key(&mut conn, &unit, &wk("first")).await.unwrap(), wk("first"));
        assert_eq!(store_wrapped_key(&mut conn, &unit, &wk("second")).await.unwrap(), wk("first"), "{unit:?}: the loser gets the winner's key");
        assert_eq!(load_wrapped_key(&mut conn, &unit).await.unwrap(), Some(wk("first")));
    }
}

#[tokio::test]
async fn messages_have_no_data_key() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    assert!(matches!(load_wrapped_key(&mut conn, &Unit::Messages { tenant: Uuid::now_v7() }).await, Err(WrappedKeyError::NoDataKey)));
}

#[tokio::test]
async fn only_transit_ciphertext_is_accepted_by_the_schema() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let fau = active_fau(&db.app_pool().await, "admin@example.test", at(T0)).await;
    let bad = sqlx::query("insert into wrapped_keys (tenant_id, unit, scope, wrapped_key) values ($1, 'record', null, $2)")
        .bind(fau.tenant_id).bind(b"raw key bytes".to_vec()).execute(&pool).await;
    assert!(bad.is_err(), "a value without the vault:v prefix must be refused");
    let bad_scope = sqlx::query("insert into wrapped_keys (tenant_id, unit, scope, wrapped_key) values ($1, 'record', 'x', $2)")
        .bind(fau.tenant_id).bind(b"vault:v1:abc".to_vec()).execute(&pool).await;
    assert!(bad_scope.is_err(), "a record key has no scope");
}
```

In `crates/app/tests/schema_review.rs`, in the forbidden-column test, skip one allowlisted
column before the substring check:

```rust
    // The one deliberate exception (docs/key-service-design.md §3.2): OpenBao-wrapped data
    // keys, useless without their transit key.
    const ALLOWED: &[&str] = &["wrapped_keys.wrapped_key"];
    let mut offenders = Vec::new();
    for (table, column) in &columns {
        if ALLOWED.contains(&format!("{table}.{column}").as_str()) {
            continue;
        }
        let c = column.to_ascii_lowercase();
```

Add a test that the exception stays exactly one:

```rust
#[test]
fn the_key_material_exception_list_is_exactly_one_column() {
    let source = include_str!("schema_review.rs");
    assert_eq!(source.matches("const ALLOWED: &[&str] = &[\"wrapped_keys.wrapped_key\"];").count(), 1);
}
```

In `crates/app/tests/membership_schema.rs`:
- in `access_request_sealed_message_is_bounded`, rename the test to
  `access_request_encrypted_message_is_bounded`, the column to `encrypted_message`, the bound
  values to `4096` / `4097`, and the constraint name to
  `access_request_encrypted_message_is_bounded`;
- in `invitation_encrypted_message_is_bounded`, the bound values become `4096` / `4097`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-app --test wrapped_keys --test schema_review --test membership_schema`
Expected: `wrapped_keys` does not compile. `membership_schema` fails on the old column name.

- [ ] **Step 3: Write `migrations/0005_wrapped_keys.sql`**

```sql
-- #3506 (docs/key-service-design.md §3.2): OpenBao-wrapped data keys, one per unit. A wrapped
-- key is useless without its transit key in OpenBao, which exists at exactly the granularity
-- it must be shredded at, so a copy in any backup of this table is useless once that key is
-- destroyed. No usable key material is ever stored here.
create table wrapped_keys (
  tenant_id    uuid        not null references tenants (id),
  unit         text        not null check (unit in ('record', 'document', 'chat')),
  -- Document id or YYYY-MM; null for 'record'. Not `subject`, which ADR-003 decision 3
  -- reserves for identity_mappings.
  scope        text,
  -- OpenBao transit ciphertext, `vault:v<n>:<base64>`.
  wrapped_key  bytea       not null,
  created_at   timestamptz not null default now(),
  constraint wrapped_keys_scope_matches_unit check ((unit = 'record') = (scope is null)),
  constraint wrapped_keys_is_transit_ciphertext
    check (substring(wrapped_key from 1 for 7) = 'vault:v'::bytea and octet_length(wrapped_key) <= 512)
);
create unique index wrapped_keys_one_per_unit on wrapped_keys (tenant_id, unit, coalesce(scope, ''));
grant select, insert, delete on wrapped_keys to fau_app;

-- The #3418 messages are now OpenBao transit ciphertext (vault:v1: + base64), about 2,713 bytes
-- for 500 characters, so the 0003 bound of 2200 no longer fits.
alter table access_requests rename column sealed_message to encrypted_message;
alter table access_requests drop constraint access_request_sealed_message_is_bounded;
alter table access_requests add constraint access_request_encrypted_message_is_bounded
  check (octet_length(encrypted_message) <= 4096);
alter table invitations drop constraint invitation_encrypted_message_is_bounded;
alter table invitations add constraint invitation_encrypted_message_is_bounded
  check (octet_length(encrypted_message) <= 4096);

insert into schema_contract (version) values (5);
```

Check the exact name of 0003's grants to `fau_app` on `access_requests` and `invitations`. A
renamed column keeps its grants, so no new grant is needed there. Leave
`MINIMUM_CONTRACT_VERSION` at 2: the app reads no 0005 object that the startup gate must insist
on.

- [ ] **Step 4: Write `crates/persistence/src/keys.rs`**

```rust
//! Wrapped data keys (docs/key-service-design.md §3.2). Callers pass their own
//! connection, so a unit's key is stored in the same transaction as the first row it
//! protects.

use fau_crypto::{Unit, WrappedKey};
use sqlx::PgConnection;

#[derive(Debug, thiserror::Error)]
pub enum WrappedKeyError {
    #[error("this unit has no data key")]
    NoDataKey,
    #[error("a stored wrapped key is not transit ciphertext")]
    Corrupt,
    #[error("database error")]
    Db(#[from] sqlx::Error),
}

fn row(unit: &Unit) -> Result<(&'static str, Option<String>), WrappedKeyError> {
    unit.storage().ok_or(WrappedKeyError::NoDataKey)
}

fn decode(bytes: Vec<u8>) -> Result<WrappedKey, WrappedKeyError> {
    WrappedKey::new(String::from_utf8(bytes).map_err(|_| WrappedKeyError::Corrupt)?).ok_or(WrappedKeyError::Corrupt)
}

pub async fn load_wrapped_key(conn: &mut PgConnection, unit: &Unit) -> Result<Option<WrappedKey>, WrappedKeyError> {
    let (u, scope) = row(unit)?;
    let found: Option<Vec<u8>> = sqlx::query_scalar(
        "select wrapped_key from wrapped_keys where tenant_id = $1 and unit = $2 and coalesce(scope, '') = coalesce($3, '')")
        .bind(unit.tenant()).bind(u).bind(scope).fetch_optional(&mut *conn).await?;
    found.map(decode).transpose()
}

pub async fn store_wrapped_key(conn: &mut PgConnection, unit: &Unit, key: &WrappedKey) -> Result<WrappedKey, WrappedKeyError> {
    let (u, scope) = row(unit)?;
    sqlx::query(
        "insert into wrapped_keys (tenant_id, unit, scope, wrapped_key) values ($1, $2, $3, $4)
         on conflict (tenant_id, unit, coalesce(scope, '')) do nothing")
        .bind(unit.tenant()).bind(u).bind(&scope).bind(key.as_str().as_bytes())
        .execute(&mut *conn).await?;
    load_wrapped_key(conn, unit).await?.ok_or(WrappedKeyError::Corrupt)
}
```

Add `pub mod keys;` to `crates/persistence/src/lib.rs`, and `fau-crypto` to `[dev-dependencies]`
of `fau-app`.

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test -p fau-app --test wrapped_keys --test schema_review --test membership_schema --test migrations`
Expected: all pass. `migrations` may count migration files (`migration_file_count()`), so it
picks up 0005 by itself. If a test pins the latest contract version, update it to 5.

- [ ] **Step 6: Commit**

```bash
git add backend/migrations/0005_wrapped_keys.sql backend/crates/persistence backend/crates/app/tests backend/crates/app/Cargo.toml backend/Cargo.lock
git commit -m "Add wrapped_keys and widen the message columns for transit ciphertext (#3506)"
```

---

### Task 5: `KeyCache` and the data-key flow

**Files:**
- Create: `backend/crates/keys/src/cache.rs`, `backend/crates/keys/src/data_key.rs`
- Modify: `backend/crates/keys/src/lib.rs`, `backend/crates/keys/Cargo.toml` (`fau-persistence = { path = "../persistence" }`, `jiff = { workspace = true }`, `sqlx = { workspace = true }`)
- Test: `backend/crates/keys/tests/cache.rs`

**Interfaces:**
- Consumes: `Keys` (Task 3), `load_wrapped_key` / `store_wrapped_key` (Task 4).
- Produces:
  - `CacheClock = Arc<dyn Fn() -> jiff::Timestamp + Send + Sync>`.
  - `KeyCache::new(keys: Arc<Keys>, clock: CacheClock, idle: jiff::SignedDuration)`, with methods:
    - `get(session: Uuid, unit: &Unit, wrapped: &WrappedKey) -> Result<DataKey, KeyError>`: cached, or unwrapped once; **counts as activity**;
    - `put(session: Uuid, unit: Unit, key: DataKey)`;
    - `touch_session(session)`: **only** for real user requests (middleware, #3417);
    - `release_session(session)`;
    - `release_document(session, tenant, document)`;
    - `sweep() -> usize`: drops entries idle ≥ `idle`; not activity;
    - `len()`.
  - `data_key(conn: &mut PgConnection, keys: &Keys, cache: &KeyCache, session: Uuid, unit: &Unit) -> Result<DataKey, DataKeyError>`:
    - it loads the unit's wrapped key;
    - if there is none, it creates one in OpenBao and stores it (the first writer wins, and a
      loser uses the winner's key);
    - it returns the plaintext through the cache.
  - `DataKeyError { Keys(KeyError), Store(WrappedKeyError) }`.

Dropping an entry zeroises its key, because `DataKey` is `Zeroizing`. Nothing else needs doing on
release: there are no leases to hand back (plan header).

- [ ] **Step 1: Write the failing tests** (`tests/cache.rs`)

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fau_crypto::Unit;
use fau_keys::{Auth, CacheClock, KeyCache, KeyError, Keys, KeysConfig};
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

async fn keys() -> Arc<Keys> {
    Arc::new(Keys::connect(KeysConfig {
        address: std::env::var("TEST_OPENBAO_ADDR").unwrap(), ca_cert_path: None,
        auth: Auth::Token("dev-only-app".into()), timeout: Duration::from_secs(5),
    }).await.unwrap())
}

fn clock() -> (Arc<Mutex<Timestamp>>, CacheClock) {
    let t = Arc::new(Mutex::new(Timestamp::now()));
    let t2 = t.clone();
    (t, Arc::new(move || *t2.lock().unwrap()))
}

fn advance(t: &Arc<Mutex<Timestamp>>, mins: i64) {
    let mut g = t.lock().unwrap();
    *g = *g + SignedDuration::from_mins(mins);
}

async fn soft_delete(unit: &Unit) {
    let addr = std::env::var("TEST_OPENBAO_ADDR").unwrap();
    let root = std::env::var("TEST_OPENBAO_TOKEN").unwrap();
    let s = reqwest::Client::new().delete(format!("{addr}/v1/transit/keys/{}/soft-delete", unit.key_name())).header("X-Vault-Token", root).send().await.unwrap().status();
    assert!(s.is_success());
}

#[tokio::test]
async fn a_session_unwraps_once_and_then_serves_from_memory() {
    let keys = keys().await;
    let (_t, c) = clock();
    let cache = KeyCache::new(keys.clone(), c, SignedDuration::from_mins(30));
    let unit = Unit::Record { tenant: Uuid::now_v7() };
    let (key, wrapped) = keys.new_data_key(&unit).await.unwrap();
    let s = Uuid::now_v7();
    assert_eq!(cache.get(s, &unit, &wrapped).await.unwrap().expose(), key.expose());
    soft_delete(&unit).await;
    assert_eq!(cache.get(s, &unit, &wrapped).await.unwrap().expose(), key.expose(), "served from memory: no second unwrap");
    assert_eq!(cache.get(Uuid::now_v7(), &unit, &wrapped).await.unwrap_err(), KeyError::NotFound, "a new session must unwrap, and the key is gone");
}

#[tokio::test]
async fn idle_sessions_are_swept_sweeping_is_not_activity_and_touching_is() {
    let keys = keys().await;
    let (t, c) = clock();
    let cache = KeyCache::new(keys.clone(), c, SignedDuration::from_mins(30));
    let unit = Unit::Record { tenant: Uuid::now_v7() };
    let (key, _) = keys.new_data_key(&unit).await.unwrap();
    let (idle, busy) = (Uuid::now_v7(), Uuid::now_v7());
    cache.put(idle, unit, key.clone());
    cache.put(busy, unit, key);
    advance(&t, 29);
    assert_eq!(cache.sweep(), 0);
    cache.touch_session(busy);
    advance(&t, 2);
    assert_eq!(cache.sweep(), 1, "idle for 31 minutes; the sweep at 29 did not refresh it");
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
    cache.put(a, Unit::Document { tenant, document: d1 }, k.clone());
    cache.put(a, Unit::Document { tenant, document: d2 }, k.clone());
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
    cache.put(Uuid::now_v7(), Unit::Record { tenant: Uuid::now_v7() }, fau_crypto::DataKey::from_bytes([0xAB; 32]));
    let dbg = format!("{cache:?}");
    assert!(!dbg.contains("171") && !dbg.to_lowercase().contains("ab, ab"), "{dbg}");
}
```

`data_key` is tested end to end in Task 7, where Postgres and OpenBao are both present. Add
`reqwest` to `fau-keys`' dev-dependencies if it is not already there (it is, from Task 3).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-keys --test cache`
Expected: compile errors.

- [ ] **Step 3: Implement `src/cache.rs`**

```rust
//! Session-held data keys (ADR-003 decision 5a; docs/key-service-design.md §3.2). Memory
//! only. `DataKey` zeroises on drop, so removing an entry is releasing it. Only real user
//! requests count as activity.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use fau_crypto::{DataKey, Unit, WrappedKey};
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

use crate::client::{KeyError, Keys};

pub type CacheClock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

struct Entry { key: DataKey, last_activity: Timestamp }

pub struct KeyCache {
    keys: Arc<Keys>,
    clock: CacheClock,
    idle: SignedDuration,
    entries: Mutex<HashMap<(Uuid, Unit), Entry>>,
}

impl fmt::Debug for KeyCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyCache").field("entries", &self.len()).field("idle", &self.idle).finish()
    }
}

impl KeyCache {
    pub fn new(keys: Arc<Keys>, clock: CacheClock, idle: SignedDuration) -> Self {
        Self { keys, clock, idle, entries: Mutex::new(HashMap::new()) }
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<(Uuid, Unit), Entry>> {
        self.entries.lock().expect("key cache lock")
    }

    pub fn len(&self) -> usize { self.map().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub async fn get(&self, session: Uuid, unit: &Unit, wrapped: &WrappedKey) -> Result<DataKey, KeyError> {
        let now = (self.clock)();
        if let Some(e) = self.map().get_mut(&(session, *unit)) {
            e.last_activity = now;
            return Ok(e.key.clone());
        }
        let key = self.keys.unwrap(unit, wrapped).await?;
        self.put(session, *unit, key.clone());
        Ok(key)
    }

    pub fn put(&self, session: Uuid, unit: Unit, key: DataKey) {
        let now = (self.clock)();
        self.map().insert((session, unit), Entry { key, last_activity: now });
    }

    pub fn touch_session(&self, session: Uuid) {
        let now = (self.clock)();
        for ((s, _), e) in self.map().iter_mut() {
            if *s == session {
                e.last_activity = now;
            }
        }
    }

    pub fn release_session(&self, session: Uuid) {
        self.map().retain(|(s, _), _| *s != session);
    }

    pub fn release_document(&self, session: Uuid, tenant: Uuid, document: Uuid) {
        let gone = Unit::Document { tenant, document };
        self.map().retain(|(s, u), _| !(*s == session && *u == gone));
    }

    /// Reads `last_activity`, never writes it.
    pub fn sweep(&self) -> usize {
        let now = (self.clock)();
        let mut map = self.map();
        let before = map.len();
        map.retain(|_, e| now.duration_since(e.last_activity) < self.idle);
        before - map.len()
    }
}
```

`src/data_key.rs`:

```rust
//! The data-key flow (docs/key-service-design.md §3.2): load the unit's wrapped key, or
//! create one; return the plaintext through the session cache.

use fau_crypto::{DataKey, Unit};
use fau_persistence::keys::{load_wrapped_key, store_wrapped_key, WrappedKeyError};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::cache::KeyCache;
use crate::client::{KeyError, Keys};

#[derive(Debug, thiserror::Error)]
pub enum DataKeyError {
    #[error(transparent)] Keys(#[from] KeyError),
    #[error(transparent)] Store(#[from] WrappedKeyError),
}

pub async fn data_key(conn: &mut PgConnection, keys: &Keys, cache: &KeyCache, session: Uuid, unit: &Unit) -> Result<DataKey, DataKeyError> {
    if let Some(wrapped) = load_wrapped_key(conn, unit).await? {
        return Ok(cache.get(session, unit, &wrapped).await?);
    }
    let (fresh, wrapped) = keys.new_data_key(unit).await?;
    let stored = store_wrapped_key(conn, unit, &wrapped).await?;
    if stored == wrapped {
        cache.put(session, *unit, fresh.clone());
        Ok(fresh)
    } else {
        // Another session stored its key first: ours is discarded and never used.
        Ok(cache.get(session, unit, &stored).await?)
    }
}
```

`src/lib.rs`:

```rust
//! FAU's use of OpenBao (docs/key-service-design.md).

mod cache;
mod client;
mod data_key;

pub use cache::{CacheClock, KeyCache};
pub use client::{Auth, KeyError, Keys, KeysConfig};
pub use data_key::{data_key, DataKeyError};
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p fau-keys`
Expected: Task 3's tests plus 4 cache tests pass.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/keys backend/Cargo.lock
git commit -m "Add the session key cache and the data-key flow (#3506)"
```

---

### Task 6: The two #3418 messages

This ends #3418's "accept no message until the key service exists" deferral. Persistence stores
OpenBao transit ciphertext and never sees plaintext. The HTTP layer (#3417) will call
`Keys::encrypt_message` / `decrypt_message`. The message carries the id of the row it is bound
to, because the associated data names that row.

**Files:**
- Modify: `backend/crates/persistence/src/membership/{requests,invitations,handover,signup,mod}.rs`
- Modify: every `CreateAccessRequest { .. }` and `IssueInvitation { .. }` literal under `crates/app/tests` (`grep -rn "CreateAccessRequest {\|IssueInvitation {" backend/crates/app/tests`), adding `message: None`
- Test: `backend/crates/app/tests/requests.rs`, `backend/crates/app/tests/invitations.rs`

**Interfaces:**
- Consumes: `fau_crypto::MessageCiphertext`, `fau_crypto::Aad`.
- Produces:
  - `AccessRequestMessage { request_id: Uuid, ciphertext: MessageCiphertext }`, with `CreateAccessRequest.message: Option<AccessRequestMessage>`.
  - `access_request_message(pool, tenant_id, request_id, actor_membership_id, at: Moment) -> Result<Option<MessageCiphertext>, MembershipError>`. An admin valid today is checked **before** the row is read.
  - `InvitationMessage { invitation_id: Uuid, ciphertext: MessageCiphertext }`, with `IssueInvitation.message: Option<InvitationMessage>`.
  - `invitation_message(pool, token: &str, reader: &VerifiedEmail, at: Moment) -> Result<Option<InvitationMessageView>, MembershipError>`, with `InvitationMessageView { tenant_id, invitation_id, ciphertext: MessageCiphertext }`. A malformed, unknown, accepted, revoked or expired token, or a reader who is not the recipient, all give `UnknownInvitation`, and the answer never says which.
  - `ACCESS_REQUEST_MESSAGE_AAD: (&str, &str) = ("access_requests", "encrypted_message")`.
  - `INVITATION_MESSAGE_AAD: (&str, &str) = ("invitations", "encrypted_message")`.
  - `MESSAGE_MAX_BYTES: usize = 4096`.

- [ ] **Step 1: Write the failing tests**

In `requests.rs`, the helper gets `message: None`. The old assertion on `sealed_message` becomes:

```rust
    let stored: Option<Vec<u8>> =
        sqlx::query_scalar("select encrypted_message from access_requests where id = $1")
            .bind(id).fetch_one(&pool).await.unwrap();
    assert!(stored.is_none(), "no message given, none stored");
```

Append:

```rust
use fau_crypto::MessageCiphertext;
use fau_persistence::membership::{access_request_message, AccessRequestMessage};

fn ct(s: &str) -> MessageCiphertext { MessageCiphertext::new(format!("vault:v1:{s}")).unwrap() }

#[tokio::test]
async fn a_message_is_stored_as_given_and_read_back_only_by_an_admin() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let request_id = Uuid::now_v7();
    let id = create_access_request(&pool, CreateAccessRequest {
        tenant_id: fau.tenant_id, requester: verified("ny@example.test"),
        message: Some(AccessRequestMessage { request_id, ciphertext: ct("c2VhbGVk") }),
    }, t0).await.unwrap();
    assert_eq!(id, request_id, "the row takes the id the message is bound to");
    assert_eq!(access_request_message(&pool, fau.tenant_id, id, fau.admin_membership_id, t0).await.unwrap(), Some(ct("c2VhbGVk")));

    let member = add_member(&pool, &fau, "m@example.test", new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2027, 9, 1)), t0).await;
    for request in [id, Uuid::now_v7()] {
        assert!(matches!(access_request_message(&pool, fau.tenant_id, request, member.membership_id, t0).await,
            Err(MembershipError::NotAuthorized)), "authority is checked before the row");
    }
}

#[tokio::test]
async fn an_oversized_message_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let r = create_access_request(&pool, CreateAccessRequest {
        tenant_id: fau.tenant_id, requester: verified("ny@example.test"),
        message: Some(AccessRequestMessage { request_id: Uuid::now_v7(), ciphertext: ct(&"A".repeat(4096)) }),
    }, t0).await;
    assert!(matches!(r, Err(MembershipError::MessageTooLong)));
}
```

In `invitations.rs`, add `message: None` to every literal and append:

```rust
use fau_crypto::MessageCiphertext;
use fau_persistence::membership::{invitation_message, resend_invitation, InvitationChange, InvitationMessage};

async fn invite_with(pool: &sqlx::PgPool, fau: &Fau, to: &str, t: fau_domain::time::Moment) -> fau_persistence::membership::IssuedInvitation {
    let invitation_id = Uuid::now_v7();
    let issued = issue_invitation(pool, IssueInvitation {
        tenant_id: fau.tenant_id, actor_membership_id: fau.admin_membership_id, recipient: email(to),
        roles: vec![OfferedRole { role: new_role("Medlem", CapabilityClass::Member), period: period(day(2026, 9, 1), day(2027, 9, 1)) }],
        handover_grant_id: None,
        message: Some(InvitationMessage { invitation_id, ciphertext: MessageCiphertext::new("vault:v1:bWVsZGluZw==".into()).unwrap() }),
    }, t).await.unwrap();
    assert_eq!(issued.invitation_id, invitation_id);
    issued
}

#[tokio::test]
async fn only_the_recipient_with_a_live_token_reads_the_message() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite_with(&pool, &fau, "ny@example.test", t0).await;
    let token = issued.token.expose().to_owned();
    let view = invitation_message(&pool, &token, &verified("ny@example.test"), t0).await.unwrap().unwrap();
    assert_eq!((view.tenant_id, view.invitation_id, view.ciphertext.as_str()), (fau.tenant_id, issued.invitation_id, "vault:v1:bWVsZGluZw=="));
    for (tok, reader) in [(token.as_str(), "annen@example.test"), (&"0".repeat(64)[..], "ny@example.test"), ("short", "ny@example.test")] {
        assert!(matches!(invitation_message(&pool, tok, &verified(reader), t0).await, Err(MembershipError::UnknownInvitation)), "{reader}");
    }
    assert!(matches!(invitation_message(&pool, &token, &verified("ny@example.test"), at("2027-01-01T00:00:00Z")).await, Err(MembershipError::UnknownInvitation)), "expired");
}

#[tokio::test]
async fn resend_keeps_the_message_and_acceptance_ends_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let issued = invite_with(&pool, &fau, "ny@example.test", t0).await;
    let resent = resend_invitation(&pool, InvitationChange {
        tenant_id: fau.tenant_id, actor_membership_id: fau.admin_membership_id, invitation_id: issued.invitation_id,
    }, t0).await.unwrap();
    let token = resent.token.expose().to_owned();
    assert!(invitation_message(&pool, &token, &verified("ny@example.test"), t0).await.unwrap().is_some());
    accept_invitation(&pool, AcceptInvitation { token: token.clone(), acceptor: verified("ny@example.test"), admin_end_override: None }, t0).await.unwrap();
    assert!(matches!(invitation_message(&pool, &token, &verified("ny@example.test"), t0).await, Err(MembershipError::UnknownInvitation)));
}
```

`resend_invitation(pool, InvitationChange { tenant_id, actor_membership_id, invitation_id }, at)`
returns an `IssuedInvitation` with the new token (checked against `invitations.rs` on
27 September). `invitations.rs` already imports `InvitationChange`; if not, add it to the `use`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-app --test requests --test invitations`
Expected: compile errors (`message` field, the readers).

- [ ] **Step 3: Implement in `requests.rs`**

```rust
/// The message, when given, is OpenBao transit ciphertext produced with
/// `Aad::new(tenant_id, ACCESS_REQUEST_MESSAGE_AAD.0, ACCESS_REQUEST_MESSAGE_AAD.1,
/// message.request_id)` (docs/key-service-design.md §3.3). Persistence never sees plaintext.
#[derive(Clone)]
pub struct CreateAccessRequest {
    pub tenant_id: Uuid,
    pub requester: VerifiedEmail,
    pub message: Option<AccessRequestMessage>,
}

#[derive(Debug, Clone)]
pub struct AccessRequestMessage {
    pub request_id: Uuid,
    pub ciphertext: fau_crypto::MessageCiphertext,
}

pub const ACCESS_REQUEST_MESSAGE_AAD: (&str, &str) = ("access_requests", "encrypted_message");
/// The `*_encrypted_message_is_bounded` checks in migration 0005.
pub const MESSAGE_MAX_BYTES: usize = 4096;
```

- Extend the hand-written `Debug` with
  `.field("message", &self.message.as_ref().map(|_| "[encrypted]"))`.
- `NewRequest` gains `id: Option<Uuid>` and `encrypted_message: Option<&'a [u8]>`.
- In `insert_request`:
  - use `let id = r.id.unwrap_or_else(Uuid::now_v7);`;
  - add `encrypted_message` to the column list, with `$12` bound to `.bind(r.encrypted_message)`.
- In `create_access_request`, before `lock_tenant`:

```rust
    if req.message.as_ref().is_some_and(|m| m.ciphertext.as_str().len() > MESSAGE_MAX_BYTES) {
        return Err(MembershipError::MessageTooLong);
    }
```

and in its `NewRequest` literal:

```rust
            id: req.message.as_ref().map(|m| m.request_id),
            encrypted_message: req.message.as_ref().map(|m| m.ciphertext.as_str().as_bytes()),
```

The replacement proposal's `NewRequest` gets `id: None, encrypted_message: None`. Its doc line
"No message field until the key service exists" becomes "Carries no message (flow spec §5.3)."

Append the reader:

```rust
/// The message on a request, for the approval screen (flow spec §5.2). Authority first.
pub async fn access_request_message(
    pool: &PgPool,
    tenant_id: Uuid,
    request_id: Uuid,
    actor_membership_id: Uuid,
    at: Moment,
) -> Result<Option<fau_crypto::MessageCiphertext>, MembershipError> {
    let mut tx = pool.begin().await?;
    require_admin(&mut tx, tenant_id, actor_membership_id, at.today()).await?;
    let row: Option<Option<Vec<u8>>> = sqlx::query_scalar(
        "select encrypted_message from access_requests where tenant_id = $1 and id = $2")
        .bind(tenant_id).bind(request_id).fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    row.flatten()
        .map(|b| String::from_utf8(b).ok().and_then(fau_crypto::MessageCiphertext::new).ok_or_else(MembershipError::decode))
        .transpose()
}
```

- [ ] **Step 4: Implement in `invitations.rs`**

- `IssueInvitation` gains `pub message: Option<InvitationMessage>`.
- Add:

```rust
/// OpenBao transit ciphertext produced with `Aad::new(tenant_id, INVITATION_MESSAGE_AAD.0,
/// INVITATION_MESSAGE_AAD.1, invitation_id)`, and the invitation row it is bound to.
#[derive(Debug, Clone)]
pub struct InvitationMessage {
    pub invitation_id: Uuid,
    pub ciphertext: fau_crypto::MessageCiphertext,
}

pub const INVITATION_MESSAGE_AAD: (&str, &str) = ("invitations", "encrypted_message");

#[derive(Debug, Clone)]
pub struct InvitationMessageView {
    pub tenant_id: Uuid,
    pub invitation_id: Uuid,
    pub ciphertext: fau_crypto::MessageCiphertext,
}
```

- `NewInvitation` gains `pub(crate) id: Option<Uuid>` and `pub(crate) encrypted_message: Option<Vec<u8>>`.
- `insert_invitation`:
  - uses `let invitation_id = new.id.unwrap_or_else(Uuid::now_v7);`;
  - adds the `encrypted_message` column, with `$12` bound to `.bind(new.encrypted_message)`.
- `issue_invitation`:
  - checks `MESSAGE_MAX_BYTES` first (import it from `super::requests`);
  - passes `id: req.message.as_ref().map(|m| m.invitation_id)` and
    `encrypted_message: req.message.as_ref().map(|m| m.ciphertext.as_str().as_bytes().to_vec())`.
- The three other `NewInvitation` literals (`handover.rs`, `signup.rs`, `requests.rs`) get
  `id: None, encrypted_message: None`.

Append the reader:

```rust
/// The invitee's view of the message (decision of 24 September 2026). Every failure is
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
    message
        .map(|b| {
            let ciphertext = String::from_utf8(b).ok().and_then(fau_crypto::MessageCiphertext::new).ok_or_else(MembershipError::decode)?;
            Ok(InvitationMessageView { tenant_id, invitation_id, ciphertext })
        })
        .transpose()
}
```

Export from `membership/mod.rs`:
- `access_request_message`, `AccessRequestMessage`, `ACCESS_REQUEST_MESSAGE_AAD` and `MESSAGE_MAX_BYTES` from `requests`;
- `invitation_message`, `InvitationMessage`, `InvitationMessageView` and `INVITATION_MESSAGE_AAD` from `invitations`.

- [ ] **Step 5: Run to verify they pass, then the whole workspace**

```bash
cargo test -p fau-app --test requests --test invitations
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: green. Fix any other literal the compiler names.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/persistence backend/crates/app
git commit -m "Accept the access-request and invitation messages as transit ciphertext (#3418, #3506)"
```

---

### Task 7: `content_unavailable` at the edge, and the chain end to end

**Files:**
- Modify: `backend/crates/domain/src/error_code.rs`
- Modify: `backend/crates/app/Cargo.toml` (`[dependencies] fau-keys = { path = "../keys" }`; `[dev-dependencies] fau-crypto`, `fau-keys`)
- Modify: `backend/crates/app/src/http/error.rs`
- Create: `backend/crates/app/tests/key_chain.rs`

**Interfaces:**
- Produces:
  - `ErrorCode::ContentUnavailable` (`"content_unavailable"`);
  - `ApiError::content_unavailable()` (503);
  - `impl From<fau_keys::KeyError> for ApiError`.

| `KeyError` | `ApiError` | Log |
|---|---|---|
| `Sealed`, `Unavailable`, `RateLimited` | `content_unavailable` (503) | `WARN` with the variant |
| `NotFound` | `not_found` (404): shredded or never created | none |
| `Forbidden`, `Invalid` | `internal_error` (500): our own bug or misconfiguration | `ERROR` |

- [ ] **Step 1: Write the failing tests**

In `error_code.rs` tests:

```rust
    #[test]
    fn content_unavailable_serialises_as_snake_case() {
        assert_eq!(serde_json::to_string(&ErrorCode::ContentUnavailable).unwrap(), "\"content_unavailable\"");
    }
```

In `http/error.rs`:

```rust
#[cfg(test)]
mod key_error_tests {
    use super::*;
    use fau_keys::KeyError;

    #[test]
    fn a_sealed_openbao_is_content_unavailable() {
        for (e, status) in [
            (KeyError::Sealed, StatusCode::SERVICE_UNAVAILABLE),
            (KeyError::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
            (KeyError::RateLimited, StatusCode::SERVICE_UNAVAILABLE),
            (KeyError::NotFound, StatusCode::NOT_FOUND),
            (KeyError::Forbidden, StatusCode::INTERNAL_SERVER_ERROR),
            (KeyError::Invalid, StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            assert_eq!(ApiError::from(e).status, status, "{e:?}");
        }
        assert_eq!(ApiError::from(KeyError::Sealed).code, ErrorCode::ContentUnavailable);
    }
}
```

`tests/key_chain.rs`:

```rust
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
    Arc::new(Keys::connect(KeysConfig {
        address: std::env::var("TEST_OPENBAO_ADDR").unwrap(), ca_cert_path: None,
        auth: Auth::Token("dev-only-app".into()), timeout: Duration::from_secs(5),
    }).await.unwrap())
}

/// Stands in for the deletion job (Task 8): soft-delete every key of the FAU.
async fn soft_delete_fau(tenant: Uuid) {
    let addr = std::env::var("TEST_OPENBAO_ADDR").unwrap();
    let root = std::env::var("TEST_OPENBAO_TOKEN").unwrap();
    for name in [Unit::Record { tenant }.key_name(), Unit::Messages { tenant }.key_name()] {
        let s = reqwest::Client::new().delete(format!("{addr}/v1/transit/keys/{name}/soft-delete")).header("X-Vault-Token", &root).send().await.unwrap().status();
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
    let cache = KeyCache::new(keys.clone(), Arc::new(jiff::Timestamp::now), SignedDuration::from_mins(30));
    let record = Unit::Record { tenant: fau.tenant_id };

    // Envelope: the first session creates and stores the wrapped key; a second session unwraps it.
    let s1 = Uuid::now_v7();
    let k1 = { let mut c = pool.acquire().await.unwrap(); data_key(&mut c, &keys, &cache, s1, &record).await.unwrap() };
    let row = Uuid::now_v7();
    let aad = Aad::new(fau.tenant_id, "groups", "encrypted_name", row);
    let ct = encrypt(&k1, &aad, "Juleballkomiteen").unwrap();
    let s2 = Uuid::now_v7();
    let k2 = { let mut c = pool.acquire().await.unwrap(); data_key(&mut c, &keys, &cache, s2, &record).await.unwrap() };
    assert_eq!(decrypt(&k2, &aad, &ct).unwrap().as_str(), "Juleballkomiteen");

    // No usable key material in Postgres: the plaintext data key appears nowhere in wrapped_keys.
    let stored: Vec<u8> = sqlx::query_scalar("select wrapped_key from wrapped_keys where tenant_id = $1").bind(fau.tenant_id).fetch_one(&pool).await.unwrap();
    assert!(!stored.windows(32).any(|w| w == k1.expose()));
    assert!(stored.starts_with(b"vault:v1:"));

    // An access-request message, encrypted with no session, read on the approval screen.
    let request_id = Uuid::now_v7();
    let (t, c) = ACCESS_REQUEST_MESSAGE_AAD;
    let msg_aad = Aad::new(fau.tenant_id, t, c, request_id);
    let sealed = keys.encrypt_message(fau.tenant_id, &msg_aad, "Jeg vil bli med i FAU").await.unwrap();
    create_access_request(&pool, CreateAccessRequest { tenant_id: fau.tenant_id, requester: verified("ny@example.test"),
        message: Some(AccessRequestMessage { request_id, ciphertext: sealed }) }, t0).await.unwrap();
    let stored = access_request_message(&pool, fau.tenant_id, request_id, fau.admin_membership_id, t0).await.unwrap().unwrap();
    assert_eq!(keys.decrypt_message(fau.tenant_id, &msg_aad, &stored).await.unwrap().as_str(), "Jeg vil bli med i FAU");

    // Shredding: with the FAU's keys soft-deleted, a new session can read nothing.
    soft_delete_fau(fau.tenant_id).await;
    cache.release_session(s1);
    cache.release_session(s2);
    let s3 = Uuid::now_v7();
    let mut c = pool.acquire().await.unwrap();
    assert!(matches!(data_key(&mut c, &keys, &cache, s3, &record).await, Err(DataKeyError::Keys(KeyError::NotFound))));
    assert_eq!(keys.decrypt_message(fau.tenant_id, &msg_aad, &stored).await.unwrap_err(), KeyError::NotFound);
}
```

Add `reqwest` and `jiff` to `fau-app`'s dev-dependencies if missing (`reqwest` is there already).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p fau-domain error_code && cargo test -p fau-app --bin fau key_error_tests && cargo test -p fau-app --test key_chain`
Expected: compile errors.

- [ ] **Step 3: Implement**

`error_code.rs`:

```rust
    /// Encrypted content cannot be read right now: OpenBao is sealed, unreachable or rate
    /// limited (docs/key-service-design.md §6). Navigation, login and authorization still
    /// work. Bokmål source string for the client catalogue (#3439):
    /// "Innholdet er midlertidig utilgjengelig. Vi jobber med saken."
    ContentUnavailable,
```

`http/error.rs`: make `code` and `status` `pub(crate)` for the test, then add:

```rust
impl ApiError {
    pub fn content_unavailable() -> Self {
        Self::new(ErrorCode::ContentUnavailable, StatusCode::SERVICE_UNAVAILABLE)
    }
}

impl From<fau_keys::KeyError> for ApiError {
    fn from(e: fau_keys::KeyError) -> Self {
        use fau_keys::KeyError::*;
        match e {
            Sealed | Unavailable | RateLimited => {
                tracing::warn!(key_error = ?e, "encrypted content unavailable");
                Self::content_unavailable()
            }
            NotFound => Self::not_found(),
            Forbidden | Invalid => {
                tracing::error!(key_error = ?e, "OpenBao refused a request the backend should not have sent");
                Self::internal_error()
            }
        }
    }
}
```

- [ ] **Step 4: Run to verify they pass, then everything**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: green.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/domain backend/crates/app backend/Cargo.lock
git commit -m "Map OpenBao outages to content_unavailable and prove the key chain end to end (#3506)"
```

---

### Task 8: The deletion script (`shred.sh`) and its tests

**Files:**
- Create: `ops/openbao/shred.sh`, `ops/openbao/test-shred.sh`

**Interfaces:** the script's command line (spec §5), run under `fau-keys-operator` in the official
OpenBao image, which carries `bao` and a POSIX `sh`:

| Command | Effect | Exit |
|---|---|---|
| `shred.sh fau <tenant>` | Soft-delete every `fau-<tenant>-*` key and queue each one | 0 |
| `shred.sh document <tenant> <document>` | Soft-delete and queue `fau-<tenant>-doc-<document>` | 0 |
| `shred.sh chat-expire` | Soft-delete and queue every `fau-*-chat-YYYY-MM` whose month ended at least 12 months ago | 0 |
| `shred.sh restore <key>` | Restore a soft-deleted key and remove it from the queue | 0 |
| `shred.sh finalize` | For each queued key soft-deleted ≥ 7 days ago: confirm it is still soft-deleted, set `deletion_allowed`, delete it, unqueue it | 0 |

- Bad input gives exit 2. A refused young chat month gives exit 3.
- `SHRED_NOW` (epoch seconds) overrides the clock. **It exists for tests only.**
- Every destructive or restoring action logs one `shred:` line. Restores and refusals are
  prefixed `shred: CRITICAL` on stderr, for the alert.
- **Soft delete comes before the queue write.** A crash between the two leaves a key
  soft-deleted but unqueued: inert, restorable, and noticed by audit. The opposite order could
  queue an **active** key for destruction.
- **Chat age is computed in UTC months,** which is conservative. Near a month boundary, the UTC
  month is never later than the Oslo month, so a key is never destroyed early. This avoids
  needing tzdata in the image.

- [ ] **Step 1: Write the failing test** (`ops/openbao/test-shred.sh`)

```sh
#!/bin/sh
# Tests for shred.sh against the dev OpenBao (run inside the openbao container, Task 2).
# Root creates fixtures; shred.sh itself runs as dev-only-operator, like production.
set -eu
export BAO_ADDR="${BAO_ADDR:-http://127.0.0.1:8200}"
ROOT="${BAO_TOKEN:?root token}"
S="$(dirname "$0")/shred.sh"
fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }
mk() { BAO_TOKEN=$ROOT bao write -f "transit/keys/$1" >/dev/null; }
soft() { BAO_TOKEN=$ROOT bao read -field=soft_deleted "transit/keys/$1" 2>/dev/null || echo gone; }
run() { now="$1"; shift; BAO_TOKEN=dev-only-operator SHRED_NOW="$now" sh "$S" "$@"; }

T=$(cat /proc/sys/kernel/random/uuid); D=$(cat /proc/sys/kernel/random/uuid)
NOW=1790500000                         # 2026-09-27T10:26:40Z
DAY=86400
mk "fau-$T-record"; mk "fau-$T-messages"; mk "fau-$T-doc-$D"; mk "fau-$T-chat-2026-09"
OTHER=$(cat /proc/sys/kernel/random/uuid); mk "fau-$OTHER-record"

run $NOW document "$T" "$D"
[ "$(soft fau-$T-doc-$D)" = "true" ] && [ "$(soft fau-$T-record)" = "false" ] || fail "document soft-deletes exactly one key"
pass "document"

run $NOW restore "fau-$T-doc-$D" 2>/dev/null
[ "$(soft fau-$T-doc-$D)" = "false" ] || fail "restore"
BAO_TOKEN=$ROOT bao read "fau-keys-queue/fau-$T-doc-$D" >/dev/null 2>&1 && fail "restore unqueues"
pass "restore"

run $NOW fau "$T"
for k in record messages "doc-$D" chat-2026-09; do [ "$(soft fau-$T-$k)" = "true" ] || fail "fau soft-deletes fau-$T-$k"; done
[ "$(soft fau-$OTHER-record)" = "false" ] || fail "fau leaves other FAUs alone"
pass "fau, including a young chat month (governed deletion, not expiry)"

run $((NOW + 6*DAY)) finalize
[ "$(soft fau-$T-record)" = "true" ] || fail "nothing destroyed before 7 days"
run $((NOW + 7*DAY)) finalize
[ "$(soft fau-$T-record)" = "gone" ] && [ "$(soft fau-$T-chat-2026-09)" = "gone" ] || fail "destroyed after 7 days"
[ "$(soft fau-$OTHER-record)" = "false" ] || fail "finalize leaves unqueued keys alone"
pass "finalize after 7 days"

C=$(cat /proc/sys/kernel/random/uuid)
mk "fau-$C-chat-2025-08"; mk "fau-$C-chat-2025-09"; mk "fau-$C-chat-2026-09"
run $NOW chat-expire                   # current UTC month 2026-09 (index diff: 2025-08 -> 13)
[ "$(soft fau-$C-chat-2025-08)" = "true" ] || fail "a month that ended 12+ months ago expires"
[ "$(soft fau-$C-chat-2025-09)" = "false" ] && [ "$(soft fau-$C-chat-2026-09)" = "false" ] || fail "younger months are kept"
pass "chat-expire"

# A queued key that someone restored by hand must not be destroyed by finalize.
R=$(cat /proc/sys/kernel/random/uuid); mk "fau-$R-record"
run $NOW fau "$R"
BAO_TOKEN=$ROOT bao write -f "transit/keys/fau-$R-record/soft-delete-restore" >/dev/null
run $((NOW + 8*DAY)) finalize
[ "$(soft fau-$R-record)" = "false" ] || fail "finalize destroyed a key that was restored out of band"
pass "finalize re-checks soft_deleted"

run $NOW fau "$T" >/dev/null           # a second run over already soft-deleted keys succeeds
pass "idempotent"
if run $NOW nonsense 2>/dev/null; then fail "unknown command must fail"; fi
pass "bad input"
echo "all shred.sh tests passed"
```

Run it:

```bash
cd /workspace
docker compose -f compose.yaml exec -T -e BAO_TOKEN=dev-only-root openbao sh /ops/test-shred.sh
```

Expected: fails, because `shred.sh` does not exist yet.

- [ ] **Step 2: Write `ops/openbao/shred.sh`**

```sh
#!/bin/sh
# FAU key deletion (docs/key-service-design.md §5). Runs as fau-keys-operator, which can
# destroy keys and never read data. See the usage line below.
set -eu
WINDOW=$((7 * 24 * 3600))              # ADR-003 decision 7
Q=fau-keys-queue

now() { echo "${SHRED_NOW:-$(date -u +%s)}"; }
log() { echo "shred: $*"; }
crit() { echo "shred: CRITICAL $*" >&2; }
usage() { echo "usage: shred.sh fau <tenant> | document <tenant> <document> | chat-expire | restore <key> | finalize" >&2; exit 2; }
is_uuid() { echo "$1" | grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'; }
# `bao list` prints a JSON array; an empty mount is an error, which means "nothing".
list() { bao list -format=json "$1" 2>/dev/null | tr -d '[]", ' | sed '/^$/d' || true; }
month_index() { y=${1%-*}; m=${1#*-}; m=${m#0}; echo $((y * 12 + m - 1)); }

soft() {
  # Idempotent: a key already soft-deleted (an earlier run, or the monthly job again) is skipped.
  [ "$(bao read -field=soft_deleted "transit/keys/$1" 2>/dev/null || echo gone)" = "false" ] || { log "skipped $1 (already soft-deleted or gone)"; return 0; }
  bao delete "transit/keys/$1/soft-delete" >/dev/null      # first: see the ordering rule
  bao write "$Q/$1" soft_deleted_at="$(now)" >/dev/null
  log "soft-deleted $1"
}

cmd="${1:-}"; [ -n "$cmd" ] || usage; shift
case "$cmd" in
  fau)
    t="${1:-}"; is_uuid "$t" || usage
    for k in $(list transit/keys); do case "$k" in "fau-$t-"*) soft "$k" ;; esac; done
    ;;
  document)
    t="${1:-}"; d="${2:-}"; is_uuid "$t" && is_uuid "$d" || usage
    soft "fau-$t-doc-$d"
    ;;
  chat-expire)
    cur=$(month_index "$(date -u -d "@$(now)" +%Y-%m)")
    for k in $(list transit/keys); do
      m="${k##*-chat-}"
      case "$k" in fau-*-chat-[0-9][0-9][0-9][0-9]-[0-9][0-9]) ;; *) continue ;; esac
      if [ $((cur - $(month_index "$m"))) -ge 13 ]; then soft "$k"; fi
    done
    ;;
  restore)
    k="${1:-}"; case "$k" in fau-*) ;; *) usage ;; esac
    bao write -f "transit/keys/$k/soft-delete-restore" >/dev/null
    bao delete "$Q/$k" >/dev/null 2>&1 || true
    crit "restored $k"
    ;;
  finalize)
    for k in $(list "$Q"); do
      at=$(bao read -field=soft_deleted_at "$Q/$k")
      [ $(( $(now) - at )) -ge $WINDOW ] || continue
      if [ "$(bao read -field=soft_deleted "transit/keys/$k" 2>/dev/null || echo gone)" != "true" ]; then
        crit "$k is queued but not soft-deleted (restored out of band?); unqueued, not destroyed"
        bao delete "$Q/$k" >/dev/null
        continue
      fi
      bao write "transit/keys/$k/config" deletion_allowed=true >/dev/null
      bao delete "transit/keys/$k" >/dev/null
      bao delete "$Q/$k" >/dev/null
      log "destroyed $k"
    done
    ;;
  *) usage ;;
esac
```

`chmod +x ops/openbao/shred.sh ops/openbao/test-shred.sh`

On month boundaries: `chat-expire` counts a month as ended 12 months ago when the difference of
month indexes is ≥ 13. That matches `ChatMonth::index()` in Task 1. `date -u -d @EPOCH` works in
busybox and GNU date alike. If this image's `date` rejects it, use `date -u -r EPOCH` and note the
image's date implementation in a comment.

- [ ] **Step 3: Run the tests**

```bash
docker compose -f compose.yaml exec -T -e BAO_TOKEN=dev-only-root openbao sh /ops/test-shred.sh
```

Expected: every line `PASS`, then `all shred.sh tests passed`. The dev server's state persists
between runs of the test, but every run uses fresh UUIDs.

- [ ] **Step 4: Commit**

```bash
git add ops/openbao/shred.sh ops/openbao/test-shred.sh
git commit -m "Add the key deletion script: soft delete, 7-day finalize, chat expiry, restore (#3506)"
```

---

### Task 9: The cluster: Helm values, TLS, NetworkPolicy, CronJobs, alerts, runbook

Nothing here is applied. Deploying is #3424's, and any cluster change needs Erik's go-ahead. The
task writes files and validates them offline.

**Files:**
- Create: `ops/openbao/kustomization.yaml`, `ops/openbao/k8s/{namespace,certificates,networkpolicy,operator,cronjobs,alerts}.yaml`, `ops/openbao/helm-values.yaml`, `ops/openbao/job.sh`
- Create: `docs/key-service-operations.md`

- [ ] **Step 1: Write `ops/openbao/helm-values.yaml`** (chart `openbao/openbao` 0.29.6)

```yaml
# docs/key-service-design.md §4.1. Standalone, one replica, Raft on hcloud-volumes, TLS,
# no UI, no injector. Raft snapshots are deliberately not configured (§4.1): #3507.
global:
  tlsDisable: false
injector:
  enabled: false
ui:
  enabled: false
server:
  image:
    repository: openbao/openbao
    tag: "2.7.0@sha256:71156a1c6623a5fa3f5e61b0c6a8ead0faf0df29a778339188443551995d1315"
  authDelegator:
    enabled: true            # lets OpenBao call TokenReview for Kubernetes auth
  dataStorage:
    enabled: true
    size: 1Gi
    storageClass: hcloud-volumes   # explicit: two default StorageClasses exist (#3488)
  auditStorage:
    enabled: false           # the audit device writes to stdout -> Loki (§4.4)
  extraVolumes:
    - type: secret
      name: openbao-server-tls
  standalone:
    enabled: true
    config: |
      ui = false
      disable_mlock = false
      listener "tcp" {
        address       = "[::]:8200"
        tls_cert_file = "/openbao/userconfig/openbao-server-tls/tls.crt"
        tls_key_file  = "/openbao/userconfig/openbao-server-tls/tls.key"
        telemetry { unauthenticated_metrics_access = true }
      }
      storage "raft" {
        path    = "/openbao/data"
        node_id = "openbao-0"
      }
      telemetry {
        prometheus_retention_time = "24h"
        disable_hostname = true
      }
```

Check the chart's value names against `helm show values openbao/openbao --version 0.29.6` where
Helm is available. It is not in the agent container (plan header), so #3424 or Erik verifies
this. `unauthenticated_metrics_access` lets Prometheus scrape `vault_core_unsealed` while OpenBao
is sealed, which is exactly when that metric matters.

- [ ] **Step 2: Write the manifests**

`ops/openbao/k8s/namespace.yaml`:

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: openbao
```

`ops/openbao/k8s/certificates.yaml` sets up the internal CA, separate from the ACME issuers
(§4.1):

```yaml
apiVersion: cert-manager.io/v1
kind: Issuer
metadata: { name: openbao-bootstrap, namespace: openbao }
spec: { selfSigned: {} }
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata: { name: openbao-internal-ca, namespace: openbao }
spec:
  isCA: true
  commonName: fau-openbao-internal-ca
  secretName: openbao-internal-ca
  duration: 87600h
  privateKey: { algorithm: ECDSA, size: 256 }
  issuerRef: { name: openbao-bootstrap, kind: Issuer }
---
apiVersion: cert-manager.io/v1
kind: Issuer
metadata: { name: openbao-internal-ca, namespace: openbao }
spec: { ca: { secretName: openbao-internal-ca } }
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata: { name: openbao-server, namespace: openbao }
spec:
  secretName: openbao-server-tls
  commonName: openbao.openbao.svc
  dnsNames: [openbao, openbao.openbao, openbao.openbao.svc, openbao.openbao.svc.cluster.local]
  duration: 2160h
  renewBefore: 360h
  privateKey: { algorithm: ECDSA, size: 256, rotationPolicy: Always }
  issuerRef: { name: openbao-internal-ca, kind: Issuer }
# The app trusts this CA through `ca.crt` of openbao-server-tls, distributed to the fau-app
# namespace by #3424 (trust-manager, or a copied ConfigMap). The app authenticates with its
# service account, not a client certificate (§4.3), so no client certificate is issued.
```

`ops/openbao/k8s/networkpolicy.yaml`:

```yaml
# The namespace names fau-app and monitoring are #3424's to confirm.
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: openbao, namespace: openbao }
spec:
  podSelector: { matchLabels: { app.kubernetes.io/name: openbao } }
  policyTypes: [Ingress, Egress]
  ingress:
    - from:
        - namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: fau-app } }
        - podSelector: { matchLabels: { app.kubernetes.io/name: fau-keys-operator } }
        - namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: monitoring } }
      ports: [{ port: 8200 }]
  egress:
    - to:
        - namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: kube-system } }
          podSelector: { matchLabels: { k8s-app: kube-dns } }
      ports: [{ port: 53, protocol: UDP }, { port: 53, protocol: TCP }]
    # TokenReview for Kubernetes auth: the API server. Narrow to its address once #3424 knows it.
    - ports: [{ port: 6443 }, { port: 443 }]
```

`ops/openbao/k8s/operator.yaml`:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata: { name: fau-keys-operator, namespace: openbao }
automountServiceAccountToken: true
```

`ops/openbao/job.sh` logs the job in with its service account, then runs `shred.sh`:

```sh
#!/bin/sh
# The deletion job's entrypoint: log in as fau-keys-operator via Kubernetes auth, then shred.
set -eu
BAO_TOKEN=$(bao write -field=token auth/kubernetes/login role=fau-keys-operator \
  jwt=@/var/run/secrets/kubernetes.io/serviceaccount/token)
export BAO_TOKEN
exec sh /ops/shred.sh "$@"
```

`ops/openbao/k8s/cronjobs.yaml`:

```yaml
# The 7-day finalize runs daily; chat expiry monthly. FAU deletion, document purge and restore
# are run as one-off Jobs from this template by an operator (docs/key-service-operations.md).
apiVersion: batch/v1
kind: CronJob
metadata: { name: fau-keys-finalize, namespace: openbao }
spec:
  schedule: "17 3 * * *"
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      backoffLimit: 0
      activeDeadlineSeconds: 600
      template:
        metadata: { labels: { app.kubernetes.io/name: fau-keys-operator } }
        spec:
          serviceAccountName: fau-keys-operator
          restartPolicy: Never
          securityContext: { runAsNonRoot: true, seccompProfile: { type: RuntimeDefault } }
          containers:
            - name: shred
              image: openbao/openbao:2.7.0@sha256:71156a1c6623a5fa3f5e61b0c6a8ead0faf0df29a778339188443551995d1315
              command: ["/bin/sh", "/ops/job.sh", "finalize"]
              env:
                - { name: BAO_ADDR, value: "https://openbao.openbao.svc:8200" }
                - { name: BAO_CACERT, value: /tls/ca.crt }
              securityContext: { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: [ALL] } }
              volumeMounts:
                - { name: ops, mountPath: /ops }
                - { name: tls, mountPath: /tls }
          volumes:
            - name: ops
              configMap: { name: fau-keys-scripts }
            - name: tls
              secret: { secretName: openbao-server-tls, items: [{ key: ca.crt, path: ca.crt }] }
---
apiVersion: batch/v1
kind: CronJob
metadata: { name: fau-keys-chat-expire, namespace: openbao }
spec:
  schedule: "41 3 2 * *"
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      backoffLimit: 0
      activeDeadlineSeconds: 600
      template:
        metadata: { labels: { app.kubernetes.io/name: fau-keys-operator } }
        spec:
          serviceAccountName: fau-keys-operator
          restartPolicy: Never
          securityContext: { runAsNonRoot: true, seccompProfile: { type: RuntimeDefault } }
          containers:
            - name: shred
              image: openbao/openbao:2.7.0@sha256:71156a1c6623a5fa3f5e61b0c6a8ead0faf0df29a778339188443551995d1315
              command: ["/bin/sh", "/ops/job.sh", "chat-expire"]
              env:
                - { name: BAO_ADDR, value: "https://openbao.openbao.svc:8200" }
                - { name: BAO_CACERT, value: /tls/ca.crt }
              securityContext: { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: [ALL] } }
              volumeMounts:
                - { name: ops, mountPath: /ops }
                - { name: tls, mountPath: /tls }
          volumes:
            - name: ops
              configMap: { name: fau-keys-scripts }
            - name: tls
              secret: { secretName: openbao-server-tls, items: [{ key: ca.crt, path: ca.crt }] }
```

`ops/openbao/k8s/alerts.yaml` holds the rules for #3442 (§4.4). The names carry no member data.

```yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata: { name: openbao, namespace: openbao }
spec:
  groups:
    - name: openbao
      rules:
        - alert: KeyServiceSealed
          expr: max(vault_core_unsealed) == 0
          for: 2m
          labels: { severity: critical }
        - alert: KeyServiceRateLimited
          expr: increase(vault_quota_rate_limit_violation[15m]) > 0
          labels: { severity: warning }
# Loki ruler rules for the audit log, which #3442 installs where Loki's ruler reads them.
# Label and field names follow the file audit device's JSON; #3442 verifies them against a real
# audit line before enabling:
#   KeyServiceManyDistinctKeysDecrypted (critical):
#     count(count by (key) (label_replace(count_over_time({namespace="openbao"} | json
#       | request_path=~"transit/decrypt/fau-.*" [1h]), "key", "$1", "request_path", "transit/decrypt/(.*)"))) > 50
#   KeyServiceSoftDeleteOrRestore (critical):
#     count_over_time({namespace="openbao"} | json | request_path=~"transit/keys/.+/(soft-delete|soft-delete-restore)" [15m]) > 0
#   KeyServiceShredCritical (critical):
#     count_over_time({namespace="openbao", container="shred"} |= "shred: CRITICAL" [15m]) > 0
```

`ops/openbao/kustomization.yaml`:

```yaml
# docs/key-service-design.md §4. #3424 deploys this together with the Helm chart
# (helm-values.yaml) and runs configure.sh cluster once, with a root token Erik generates.
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: openbao
resources:
  - k8s/namespace.yaml
  - k8s/certificates.yaml
  - k8s/networkpolicy.yaml
  - k8s/operator.yaml
  - k8s/cronjobs.yaml
  - k8s/alerts.yaml
configMapGenerator:
  - name: fau-keys-scripts
    files: [shred.sh, job.sh]
generatorOptions:
  disableNameSuffixHash: true
```

- [ ] **Step 3: Validate offline**

```bash
cd /workspace
kubectl kustomize ops/openbao > /dev/null && echo kustomize-ok
sh -n ops/openbao/shred.sh ops/openbao/job.sh ops/openbao/configure.sh && echo sh-syntax-ok
```

Expected: `kustomize-ok` and `sh-syntax-ok`. Do **not** `kubectl apply`.

- [ ] **Step 4: Write `docs/key-service-operations.md`**

Sections, in English, with these commands exactly:

1. **What it is:** OpenBao's transit engine as FAU's key service, with a pointer to
   `docs/key-service-design.md`.
2. **First start.** Erik, on his own terminal, never through an agent:

   ```bash
   kubectl -n openbao exec -it openbao-0 -- bao operator init -key-shares=1 -key-threshold=1
   ```

   - Store the unseal key in Proton Pass as "OpenBao unseal key" (#3481).
   - Use the initial root token for step 3, then revoke it.
3. **Configure** (after the first start, and again after any change to `ops/openbao/`):

   ```bash
   kubectl -n openbao port-forward svc/openbao 8200:8200 &
   export BAO_ADDR=https://127.0.0.1:8200 BAO_CACERT=<path to the CA from openbao-server-tls>
   export BAO_TOKEN=<root token>          # typed, never pasted into an agent session
   sh ops/openbao/configure.sh cluster
   bao token revoke -self
   ```

4. **Unseal after every restart:** `kubectl -n openbao exec -it openbao-0 -- bao operator unseal`,
   then paste the key.
5. **Generating a root token when one is needed:**
   `bao operator generate-root -init`, then `bao operator generate-root` with the unseal key and
   `-decode` using the OTP. Revoke it when done.
6. **What sealed means:**
   - login, authorization and navigation work;
   - every encrypted field answers `content_unavailable`;
   - access requests cannot attach a message until OpenBao is unsealed, because encrypting it
     needs OpenBao;
   - `KeyServiceSealed` pages after 2 minutes.
7. **Deleting an FAU, purging a document, restoring:** create a one-off Job from the
   `fau-keys-finalize` CronJob template, with the command
   `["/bin/sh", "/ops/job.sh", "fau", "<tenant>"]` (or `document <tenant> <doc>`, or
   `restore <key>`), for example with
   `kubectl -n openbao create job --from=cronjob/fau-keys-finalize …` followed by an edited command.
   Deleting an FAU runs only after ADR-003 7a's confirmation. Hard deletion follows by itself,
   7 days later, through the daily finalize.
8. **Until #3507: volume loss is total.** No Raft snapshots are configured, by design. Losing the
   PVC loses every key and so all content. Acceptable only while no real FAU data exists.
9. **Alerts:** the table from spec §4.4, and what to do for each.
10. **Local development:**
    - `docker compose -f compose.yaml up -d openbao && docker compose -f compose.yaml run --rm openbao-config`;
    - the dev tokens are `dev-only-root`, `dev-only-app` and `dev-only-operator`;
    - the dev server keeps nothing across restarts.

- [ ] **Step 5: Run everything once more**

```bash
cd /workspace/backend
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres TEST_OPENBAO_ADDR=http://openbao:8200 TEST_OPENBAO_TOKEN=dev-only-root cargo test --workspace
cd /workspace && docker compose -f compose.yaml exec -T -e BAO_TOKEN=dev-only-root openbao sh /ops/test-shred.sh
kubectl kustomize ops/openbao > /dev/null
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add ops/openbao docs/key-service-operations.md
git commit -m "Add OpenBao cluster manifests, Helm values, deletion CronJobs, alert rules and runbook (#3506)"
```

---

## After the last task

- Run `superpowers:requesting-code-review` over the branch, focused on the Global Constraints,
  the policy files and `shred.sh`.
- Favro #3506: post the result with test evidence, attach `docs/key-service-operations.md`, and pin
  a `👤 Needs you` item for the review. **Not Done** without Erik's review.
- Record in `/workspace/CLAUDE.md` (uncommitted, say so):
  - OpenBao is the key service;
  - `bao operator init`, `unseal` and `generate-root` are Erik's alone;
  - Helm is missing from the agent image, if that still holds.
