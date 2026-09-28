# FAU: encryption and key management, a brief for external review

Draft, 27 September 2026. Prepared for an outside security and cryptography reviewer. It
consolidates the design documents and the code merged on `main` as of commit `b47d778`. Where they
disagree, this brief says what the code does and flags the discrepancy (marked **Discrepancy**).
Where the author of this brief adds an observation that is not in the project documents, it is
marked **[Brief author's observation]**.

**Citation keys**

| Key | Source |
|---|---|
| ADR-003 | `docs/identity-and-encryption.md` (decisions numbered 1–10, including 3a, 4a, 5a, 6a, 7a, 7b) |
| KSD | `docs/key-service-design.md` (OpenBao design, agreed 27 September 2026) |
| OPS | `docs/key-service-operations.md` (runbook) |
| SDA | `docs/structured-document-architecture.md` |
| GDC | `docs/groups-directory-chat-calendar-design.md` |
| ARC | `docs/fau-archive-design.md` (proposed, #3509) |
| ING | `docs/document-ingest-worker.md` |
| PD | `docs/planning-decisions.md`, cited by entry title and date |
| code | a path under `backend/` or `ops/openbao/` on `main` |

---

## 1. What FAU is

FAU is a web application for Norwegian *foreldrerådenes arbeidsutvalg* (FAU-er): the elected
parents' councils at Norwegian schools. Their membership turns over every year, and records are
usually lost when a committee changes hands. FAU keeps documents, minutes, tasks, organisation,
groups, chat, a calendar and an audited history, so that an FAU's archive survives total turnover
of its members. **Continuity across turnover is the product.** A public school is legally required
to have an active FAU (ADR-003 §10).

- **Application:** a Bokmål web app (Nynorsk and further locales supported by the localisation
  mechanism). It is Rust-rendered HTML with htmx, plus a React/Tiptap editor page for documents
  (SDA §3).
- **Backend:** a Rust backend. It is also the collaboration authority for documents (SDA §5).
- **Storage:** PostgreSQL for application data; a private S3-compatible object store (Hetzner) for
  uploaded bytes.
- **Hosting:** self-hosted on a k3s Kubernetes cluster of three small Hetzner Cloud VMs (`cx23`)
  in the Helsinki region (hel1). Infrastructure is managed with Terraform.
- **Key service:** OpenBao (the Linux Foundation fork of HashiCorp Vault), self-hosted in the same
  cluster, using its transit secrets engine (KSD §1–2).
- **Identity:** authentication is bought from Hanko GmbH (Germany). Authorization is entirely FAU's
  own (ADR-003 §1–2).
- **Tenancy:** one tenant per FAU, identified by a UUIDv7 `tenant_id`. Every tenant table carries
  `tenant_id`, and references are composite.
- **Scale target:** about 1,000 FAU-er with about 20 members each (PD "Load testing with k6", 27
  September 2026).
- **Operator:** there is currently one human operator (Erik, the founder). Several controls below
  depend on that one person, and the documents record this as a standing risk (ADR-003 "Standing
  risks").

---

## 2. Trust model and threat model

### 2.1 We hold the keys

The documents state the premise plainly (ADR-003 "Why this document exists"):

- Authentication is passwordless by email. It proves control of a mailbox and yields no secret the
  user knows or holds, so there is nothing to derive a key from. Whoever completes an email
  challenge can be granted access by the server, so the server must be able to reach the data.
- Erik decided on 10 September 2026 that **recovery must work even when no member remains**. A
  design where only members hold keys would destroy an FAU's archive when a committee vanishes
  without handing over. Preventing that is the product.
- "So we hold the keys."

### 2.2 What encryption defends against

The public trust statement is ADR-003 "The trust model, stated in the words we will use
publicly":

> Encryption here defends against a **leak**, not against us.
>
> - A stolen database, a stolen backup, or a stolen object-storage bucket is inert on its own. The
>   keys are in none of them.
> - Reading one FAU's documents means driving the live application: seat a member, pass magic-link
>   verification, download. Then do it again for the next FAU. No step yields more than one FAU.
> - Every one of those steps is audited and notifies that FAU's members, so bulk exfiltration is
>   slow, linear in the number of FAU-er, and loud.
> - Root on a cluster node defeats all of it. This document says so rather than implying a
>   guarantee we cannot make.

**"We cannot see your data" is never written**: not in the DPA, not in privacy text, not in sales
material. The accurate phrase is "restricted and audited access" (ADR-003, same section).

### 2.3 Threat-model boundary

This boundary was set by Erik on 22 September 2026 (ADR-003 "What we defend against"; SDA §12):

> We defend against mistakes and against casual misuse by people who have legitimate access. We do
> not claim to withstand a determined attacker who is already inside, and we never describe the
> product as secure against one.

The documents draw these consequences from it:

- **It justifies** server-side sanitisation of imported, pasted and published content. The
  realistic case is an unaware parent pasting something they never inspected.
- **It scopes down** adversarial hardening against authenticated members to payload caps and rate
  limits. Malformed editor traffic is not fuzzed.
- **It leaves the main case unchanged.** The realistic malicious member is a compromised honest
  one. The second-factor gate (ADR-003 §4) and the recovery notifications (ADR-003 §10) address
  that case.

**Residual risks that are stated explicitly:**

- Root on a node defeats the design (ADR-003 trust model).
- The administrative MFA gate is not per-action step-up. It does not defend against an attacker
  already inside a fresh, MFA'd session (ADR-003 §4).
- A malicious recovery contact can seat an unauthorised person. That person has access until the
  misuse is noticed (ADR-003 §8).
- A compromised backend holds plaintext by construction (ADR-003 §5a).

### 2.4 Why not end-to-end encryption

E2E was asked about and **rejected on 22 September 2026** (ADR-003 "Why not end-to-end
encryption"; PD "End-to-end encryption rejected, and FAU is not a reporting channel", 22 September
2026). Three reasons are given, most decisive first:

1. **Continuity is incompatible with E2E.** When the last key-holder leaves, the data is gone. The
   recovery contact (ADR-003 §8) is an escrow mechanism, and escrow is what makes a system not E2E.
2. **E2E would remove protection against the likelier attacker.** Server-side validation of
   document JSON and the ingest pipeline (malware scanning, active-content stripping, conversion)
   need the server to read content. Under E2E they would move into the browser or vanish. That
   trades defence against a bad member for defence against the operator.
3. **There is nothing to derive a key from.** A password would reverse a core product decision. A
   separate passphrase is the second secret the product was designed to avoid. Passkey PRF is ruled
   out for the MVP, unevenly supported, and warned against for encryption by WebAuthn co-editors.

The documents also note an honesty problem. FAU serves the JavaScript and would hold the member key
directory, so E2E would hold only against a passive server.

A partial "sensitive document" class, client-encrypted to current members, was also dropped. The
related scope boundary: **FAU will never be a whistleblowing or reporting channel**. That removes
the use case the partial class would have served.

**Post-MVP direction, not built.** Member-held keys and an offline escrow wrap would move the model
towards "not without you". They need passkeys and their own trust-model decision (ADR-003 §5;
PD "Key-service root key held by hand; member keys later", 24 September 2026). The move to OpenBao's
flat transit keys may have removed the "several wraps per FAU key" hook this relied on; see §9.

---

## 3. Architecture overview

### 3.1 Components and data flow

```
Browser (htmx pages; React/Tiptap editor page)
   │  TLS. Sends plaintext document JSON, form fields and uploads. Never holds a key.
   ▼
Rust backend (Kubernetes namespace fau-app; service account fau-app)
   ├── authorizes every operation against Postgres (never against identity-provider claims)
   ├── holds unwrapped data keys in memory per session (KeyCache)
   ├── encrypts and decrypts locally with XChaCha20-Poly1305
   │
   ├──► PostgreSQL: ciphertext columns, plaintext metadata, wrapped_keys (wrapped data keys)
   ├──► Private S3: uploaded bytes, encrypted before upload (designed, not built)
   └──► OpenBao (namespace openbao; TLS on port 8200; Kubernetes auth)
          transit engine: one named key per shreddable unit.
          The backend uses: create key, datakey, encrypt, decrypt.
          OpenBao never sees content, except the two short inbound messages (§4.6).

Deletion job (CronJob or one-off Job, namespace openbao, service account fau-keys-operator)
   └──► OpenBao: list, read metadata, config, soft-delete, restore, delete, plus its queue KV mount.
        Never encrypt, decrypt or datakey.

Hanko (identity provider, EU): OIDC Authorization Code + PKCE. Holds an email address and
authentication material only.
```

Sources: KSD §3–5, ADR-003 §2–3 and §5a, and the code under `backend/crates/{crypto,keys}` and
`ops/openbao/`.

**Encryption is server-side.** The editor never holds a key. It sends document JSON to the backend
over TLS, and the backend encrypts before storage (ADR-003 §5a). For documents, the design goes
further (SDA §5): clients apply and rebase ProseMirror steps, and the server orders them, stores
encrypted step batches, broadcasts and authorizes. The server never applies a step, but it does
see step plaintext in transit.

### 3.2 Identity: Hanko

Hanko was decided on 22 September 2026, as the third provider after Zitadel Cloud and PropelAuth
(ADR-003 §1; PD "Identity provider changed again, to Hanko", 22 September 2026). PropelAuth was
dropped because it offers no DPA on the Free tier.

- **Hanko holds** the email address and the authentication material the user creates (passkey
  credentials, a TOTP secret).
- **Hanko does not hold** names, FAU membership, roles, permissions, content, titles, filenames or
  anything derived from them. Hanko has no organisation model to mirror membership into
  (ADR-003 §2).
- **Hosting:** Hanko is a German company hosting in the EU. Its infrastructure subprocessors include
  AWS (US-owned; data stays in the EU), Hetzner and adesso. This was accepted on 22 September 2026.
- **Protocol:** OIDC Authorization Code Flow with PKCE only. The issuer, endpoints and claim
  mappings are configuration.
- **Identity mapping:** identities map as `(issuer, subject) → internal user UUID`. The provider's
  `sub` never appears outside the mapping table, and a user may hold several mappings, so providers
  can be migrated by overlap (ADR-003 §3, §3a).
- **Login method:** a six-digit email passcode, sent by Hanko from EU infrastructure. Passkeys are
  Hanko's primary method; see §9.3 for the inconsistency over whether they ship in the MVP.

### 3.3 Where authorization happens

**The provider authenticates; FAU authorizes, always** (ADR-003 §2).

- Every authorization decision is taken per operation from FAU's own database, using the
  date-ranged tenant and role model (#3412). No decision ever reads a claim minted by the provider.
- A revoked role therefore dies immediately inside a live 30-day session (ADR-003 §4).
- One authorization function covers every read and the SSE stream (GDC §3.3).
- Capability classes include `guest`, which reaches only its own groups. This is authorization,
  not cryptography: keys are per FAU and per document, not per group (GDC §3.4).

**The key service does no membership checks.** It trusts the backend by service identity plus
limits (KSD K3). So **a compromised backend credential can decrypt any FAU's content**, subject
only to OpenBao's rate limit and to alerts (§5.9). This is by design, but it is the central trust
assumption.

**Administrative actions** (inviting members, granting or revoking roles, anything a recovery
contact initiates) are gated server-side on two checks (ADR-003 §4):

1. A second factor is enrolled, read from Hanko's `totp_enabled`, `auth_app_set_up` or
   `security_keys_enabled`.
2. The session is fresh: authenticated within 30–60 minutes. The exact threshold and the endpoint
   list are still open on #3414.

**The recovery contact** (FAU's operator, or a verified school representative) can do exactly one
thing: initiate adding a new member. It cannot read content or audit. Every such action is audited
permanently and emailed to all current members at creation and at acceptance (ADR-003 §8–10).

---

## 4. Keys

### 4.1 One transit key per shreddable unit

OpenBao's transit engine has no key hierarchy; keys are flat. Each unit that must be destroyable on
its own gets its own transit key, named so that all of an FAU's keys share a prefix (KSD §3.1). The
code builds the names in `backend/crates/crypto/src/unit.rs`, `Unit::key_name`. In the table,
`<tenant>` and `<document>` are UUIDs in hyphenated lowercase.

| Transit key name | Kind | Protects | Destroyed when |
|---|---|---|---|
| `fau-<tenant>-record` | wraps one data key | Small fields outside documents: group names, member display names and contact emails, event title/location/description, poll title/description | The FAU is deleted |
| `fau-<tenant>-doc-<document>` | wraps one data key | One document's step batches, checkpoints, title, filenames and bytes (KSD). SDA §7 adds comment bodies and quoted anchor text | The document is purged (ADR-003 §7b), or the FAU is deleted |
| `fau-<tenant>-chat-<YYYY-MM>` | wraps one data key | Chat bodies from that Europe/Oslo calendar month | 12 months after the month ends, or the FAU is deleted |
| `fau-<tenant>-messages` | encrypts directly (no data key) | Access-request and invitation messages | The FAU is deleted |
| `fau-<tenant>-export-<export-id>` (**proposed**, ARC §6) | wraps one data key | One export bundle in S3 | 90 days after creation, or the FAU is deleted |

**Discrepancy.** KSD §3.1 says the document key protects "step batches, checkpoints, title,
filenames and bytes". SDA §7 also lists "comment bodies and quoted anchor text" as encrypted. It is
not stated which key protects comment bodies. The document tier is not built yet.

**Transit key type** (KSD §3.1):

- The type is transit's default, `aes256-gcm96`. The code does not set `type`, so it gets the
  default (`backend/crates/keys/src/client.rs`, `ensure_key`).
- KSD's rationale: each transit key encrypts only a handful of values (wrapped data keys or short
  messages), so GCM's nonce limits are irrelevant.

**Discrepancy.** PD "OpenBao replaces the hand-written key service" (27 September 2026) says the
transit engine "provides ... XChaCha20-Poly1305 with associated data". In the design and the code,
transit keys are AES-256-GCM (96-bit nonce). XChaCha20-Poly1305 is used only for the backend's
local envelope (§4.3).

**Creation flags** (KSD §3.1):

- The design says every transit key is created with `exportable = false`,
  `allow_plaintext_backup = false` and `deletion_allowed = false`. Only the deletion job sets
  `deletion_allowed = true`, immediately before a hard delete.
- **Code:** `ensure_key` sets `exportable(false)` and `allow_plaintext_backup(false)` explicitly. It
  leaves `deletion_allowed` at the transit default, which is false.

**Who creates keys.** Keys are created lazily by the backend on first use (`ensure_key`, called
before every `new_data_key` and every `encrypt_message`). The operator policy cannot create keys.

**Behaviour verified against OpenBao 2.7.0** (plan `docs/superpowers/plans/2026-09-27-key-service-openbao.md`,
"Verified on 27 September 2026"; tests in `backend/crates/keys/tests/openbao.rs`):

- A soft-deleted key refuses `decrypt` and `datakey` ("refusing to use soft-deleted key").
- The app's create call on a soft-deleted key returns its metadata without restoring it.
- Hard delete requires `deletion_allowed=true` first.
- After a hard delete, a re-created key of the same name does **not** open old wrapped keys. The
  test gets a 400 authentication failure, which maps to `Invalid`.
- Transit `encrypt` under the app policy does not auto-create a missing key.

### 4.2 Envelope encryption, step by step

This covers records, documents and chat (KSD §3.2; code `backend/crates/keys/src/data_key.rs`,
`backend/crates/persistence/src/keys.rs`).

1. **Load.** The backend looks up `wrapped_keys` by `(tenant_id, unit, scope)` on the caller's
   Postgres connection.
2. **Existing unit.** If a row exists, `KeyCache::get(session, unit, wrapped)` returns the cached
   data key. On a miss it calls transit `decrypt` on `transit/decrypt/<key name>` with the wrapped
   ciphertext. The result must base64-decode to exactly 32 bytes, or it is `Invalid`. The key is
   then cached for that session.
3. **First use.**
   - The backend calls `ensure_key` (transit create, idempotent). It then calls
     `transit/datakey/plaintext/<key name>`, which returns both the plaintext 256-bit key (base64)
     and its wrapped form (`vault:v1:…`).
   - The wrapped form is inserted with `ON CONFLICT (tenant_id, unit, scope) DO NOTHING`, on the
     caller's connection. The design intends this to be the same transaction as the first row it
     protects. The row is then re-read.
   - **Race:** if another session's key won the insert, the freshly generated key is discarded and
     the stored one is unwrapped instead. The discarded wrapped key is never stored.
   - The code uses only `datakey/plaintext`, never `datakey/wrapped`. The KSD policy allows both.
4. **Encrypt or decrypt locally** with XChaCha20-Poly1305 under the data key (§4.3). OpenBao never
   sees record, document or chat content.

The wrapped data key carries no associated data or transit context binding it to its
`(tenant, unit, scope)`. The binding is implicit: only the unit's own transit key can unwrap it.

### 4.3 The local envelope format

Code: `backend/crates/crypto/src/envelope.rs`. Crate `fau-crypto`, using RustCrypto
`chacha20poly1305` 0.10, `getrandom` and `zeroize`.

```
stored value = version (1 byte, 0x01) ‖ nonce (24 bytes) ‖ XChaCha20-Poly1305 ciphertext ‖ tag (16 bytes)
```

- **Cipher:** XChaCha20-Poly1305 with a 256-bit key from OpenBao `datakey`.
- **Nonce:** 24 bytes from `getrandom::fill` (the OS CSPRNG), fresh for every encryption. There is
  no counter, and no key id inside the envelope (plan ruling 3). The unit is always known from
  context, because there is exactly one `wrapped_keys` row per unit.
- **Version byte:** checked equal to 1 on decrypt, but **not included in the AAD**.
- **Plaintext type:** the API takes and returns `&str` (UTF-8 only). Binary content such as
  uploads and step batches has no API in this crate yet.
- **Errors:** errors are deliberately coarse (`Randomness`, `Malformed`, `Decrypt`). A value that is
  too short (under 1 + 24 + 16 bytes) or has an unknown version is `Malformed`. An authentication
  failure is `Decrypt`, and invalid UTF-8 after decryption is `Malformed`.
- **Plaintext handling:** decrypted plaintext is returned as `Zeroizing<String>`.

### 4.4 AAD layout

The AAD is built by `Aad::to_bytes` in `envelope.rs`:

```
"fau-aad-v1"                       10 bytes, ASCII domain tag
tenant_id                          16 bytes, raw UUID
u32 big-endian len(table) ‖ table  table name, UTF-8
u32 big-endian len(column) ‖ column
row_id                             16 bytes, raw UUID
```

- `table` and `column` are `&'static str`, so they are fixed by code, not taken from input.
- Length prefixes prevent `("ab","c")`/`("a","bc")` collisions, and a test covers this.
- KSD §3.2 describes the AAD as `tenant ‖ table ‖ column ‖ row id`. The code adds the domain tag
  and the length prefixes.
- **Tests:** a ciphertext moved to another row, column, table or tenant fails to decrypt; tampering,
  truncation and an unknown version are refused; equal plaintexts encrypt differently.
- **What the AAD does not bind** [Brief author's observation]:
  - any revision or version of the value, so an older ciphertext for the same cell can be replayed
    by someone with database write access;
  - the unit or key identity;
  - the envelope version byte.

### 4.5 The `wrapped_keys` table

Migration `backend/migrations/0005_wrapped_keys.sql`:

| Column | Type and constraint |
|---|---|
| `tenant_id` | `uuid not null references tenants(id)` |
| `unit` | `text not null check (unit in ('record','document','chat'))` |
| `scope` | `text`: the document id or `YYYY-MM`, and null for `record`. Constraint `wrapped_keys_scope_matches_unit`: `(unit = 'record') = (scope is null)` |
| `wrapped_key` | `bytea not null`. Constraint `wrapped_keys_is_transit_ciphertext`: it must start with `vault:v` and be at most 512 bytes |
| `created_at` | `timestamptz not null default now()` |

- **Uniqueness:** `unique nulls not distinct (tenant_id, unit, scope)`. This is a unique constraint,
  not a primary key, because a primary key would force `scope not null`.
  **Discrepancy (minor):** KSD §3.2 says "the primary key is `(tenant_id, unit, scope)`, with a
  unique index for the null-scope case". The code uses one `NULLS NOT DISTINCT` unique constraint.
- **Grants:** the application role gets `select, insert, delete`, with no update.
- **Schema review:** `backend/crates/app/tests/schema_review.rs` fails on any column whose name
  contains `key`, `dek`, `kek`, `secret`, `private`, `passphrase`, `password`, `cipher`, `nonce` or
  `wrapped`. The single allow-listed exception is `wrapped_keys.wrapped_key`.

**Why a wrapped key in the application database is claimed safe** (KSD §3.2; ADR-003 §5 amended 27
September 2026). The original ADR rule forbade any key in the database, because the first design
wrapped every data key under one global master key that was never destroyed. Here the wrapping
transit key "exists at exactly the granularity it must be shredded at". Destroying it makes every
copy of the wrapped key useless, in the live database and in every backup. The rule now reads: **no
usable key material in the application database**.

### 4.6 Direct transit encryption for messages

Access-request and invitation messages are short (up to 500 characters) and rare, so they skip
envelope encryption. They are encrypted with transit `encrypt` and `decrypt` on
`fau-<tenant>-messages`, with `associated_data` set to the same AAD bytes as §4.4 (KSD §3.3; code
`Keys::encrypt_message`, `Keys::decrypt_message`).

- **AAD values:** `("access_requests", "encrypted_message")` and `("invitations",
  "encrypted_message")`, with the request or invitation id as `row_id`. Defined in
  `backend/crates/persistence/src/membership/{requests,invitations}.rs`.
- **Discrepancy (benign):** KSD §3.3 says the message AAD is `table ‖ column ‖ row id`. The code
  also binds the domain tag and the tenant id.
- **Storage:** `access_requests.encrypted_message` (renamed from `sealed_message`) and
  `invitations.encrypted_message`, both `bytea` with `octet_length ≤ 4096` (migration 0005).
  Persistence accepts only a `MessageCiphertext`, which must start with `vault:v<digits>:`.
- **Access requests:** encrypted at submission, when no member session exists. This works because
  encrypting needs no key in the backend. They are decrypted only on the admin's approval screen,
  and never for email.
- **Invitations:** decrypted on the invitation page for the logged-in recipient whose address
  matches, holding a valid, unused token. A resend keeps the message.
- **What was replaced.** This supersedes the 24 September 2026 designs: a per-FAU HPKE or
  sealed-box key pair for access requests, and per-invitation keys wrapped by a KEK.
  - The HPKE property "the submitting request cannot read back what it wrote" becomes "only the
    approval screen's code path calls decrypt, backed by audit". The documents acknowledge the
    backend held both halves of the capability anyway (KSD §3.3).
  - **Consequence:** OpenBao sees the plaintext of these two message types. They are the only
    content OpenBao sees.
- **If OpenBao is sealed,** an access request cannot attach a message (OPS §6).

### 4.7 The session `KeyCache`

Policy: ADR-003 §5a, decided 22 September 2026 and amended 27 September 2026, and KSD §3.2. Code:
`backend/crates/keys/src/cache.rs`.

**Design rules:**

- The backend unwraps a data key **once per user session per unit** and holds it in memory only.
- It is refreshed by **real user activity only**: no background timer, and no keep-alive from an
  idle tab.
- The idle timeout is "in the tens of minutes". The tests use 30 minutes; the production value is
  not yet set.
- Keys are zeroised on logout, session end and idle expiry. A document key is also released when
  the last client leaves the document.
- Keys are never written to disk, never logged and never included in a crash dump. The design
  requires swap disabled and core dumps off on the nodes.

**Why not per-operation fetching** (ADR-003 §5a):

- Autosave during live editing would mean a key-service round trip every few minutes per editor.
- The decisive argument is the audit signal. One unwrap per session per unit makes mass decryption
  stand out, whereas one per autosave buries 200 malicious unwraps among tens of thousands of
  routine ones.
- Per-operation fetching also buys no protection. A compromised backend holds plaintext by
  construction and can ask per operation just as easily.

**What the code does:**

- A `Mutex<HashMap<(session_id, Unit), Entry{DataKey, last_activity}>>`.
- `DataKey` wraps `Zeroizing<[u8; 32]>` and has a redacting `Debug`.
- `get` bumps `last_activity` on a hit. On a miss it unwraps via OpenBao and inserts the key.
- `touch_session` bumps every entry of a session.
- `release_session` drops all of a session's entries. `release_document(session, tenant, document)`
  drops one document key for one session.
- `sweep()` drops entries idle for at least `idle` and never writes `last_activity`, so sweeping is
  not activity.
- The cache is keyed per session, not per FAU or per instance: two sessions of the same FAU each
  unwrap separately.
- `get` returns a clone of the `DataKey` to the caller. The clone zeroises on drop, but clones held
  by in-flight requests are outside the cache's control. [Brief author's observation]

**The concurrency ceiling was dropped.** ADR-003 §5a (22 September 2026) specified a **concurrency
ceiling**: a cap on distinct keys one backend instance may hold, enforced by the key service. With
OpenBao this was dropped (27 September 2026). OpenBao has no notion of what a backend still holds,
and a hand-built lease table was rejected in favour of battle-proven components. The replacement is
**rate-limit quotas plus a critical alert on many distinct keys decrypted per hour** (KSD §8; PD
"OpenBao replaces the hand-written key service" lists it as an accepted loss). No ceiling exists in
the code.

**Discrepancy.** ADR-003's testing table still lists "Backend instance asked to hold more FAU keys
than the ceiling → Refused; alert raised". SDA §7 and GDC §8 still refer to "the concurrency
ceiling".

**Not wired yet** [Brief author's observation from reading `backend/crates/app/src`]:

- No production code calls `KeyCache`, `sweep`, `Keys`, `encrypt_message` or `decrypt_message`. The
  only use is the error mapping in `crates/app/src/http/error.rs`.
- There is no idle-sweep task, and the logout hook, the idle value and the Kubernetes auth
  configuration are not yet connected.
- The chain is exercised end to end only in tests (`backend/crates/app/tests/key_chain.rs`,
  `backend/crates/keys/tests/*.rs`).

### 4.8 The backend's OpenBao client

Code: `backend/crates/keys/src/client.rs`, using `vaultrs` 0.8.

- **Authentication:** in the cluster, Kubernetes auth. The pod's service-account JWT is read from
  file and exchanged at `auth/kubernetes/login` under role `fau-app`. Development and tests use a
  static token.
- **Lazy login.** `Keys::connect` builds the client without contacting OpenBao, and the first call
  logs in. The backend therefore starts and serves login and authorization while OpenBao is sealed
  or unreachable.
- **Retry.** A 403 triggers one re-login and one retry, for an expired Kubernetes-auth token (plan
  ruling 4). The Kubernetes role gives `token_ttl=1h`, `token_max_ttl=24h`.
- **TLS:** verified against a configured CA file, the internal CA (§5.3). There is no client
  certificate. The 26 September hand-written design specified mTLS; the OpenBao design uses the
  service-account identity instead (certificates.yaml comment).
- **Error mapping** (`map_err`, and `From<KeyError> for ApiError` in `crates/app/src/http/error.rs`):

| OpenBao response | `KeyError` | HTTP to the client |
|---|---|---|
| 503 | `Sealed` | 503 `content_unavailable` |
| 429 | `RateLimited` | 503 `content_unavailable` |
| transport or other failure | `Unavailable` | 503 `content_unavailable` |
| 400 containing "encryption key not found" or "soft-deleted" | `NotFound` | 404 `not_found` |
| 403 | `Forbidden` | 500 `internal_error`, logged at error |
| any other 400 (for example an AAD mismatch) | `Invalid` | 500 `internal_error`, logged at error |

- **Sealed behaviour.** While content is unavailable, navigation, login and authorization keep
  working. The Bokmål client string is "Innholdet er midlertidig utilgjengelig. Vi jobber med
  saken." (KSD §6).
- **Redaction.** Every type holding a key, token or ciphertext has a redacting `Debug`: `DataKey`,
  `WrappedKey`, `MessageCiphertext`, `Ciphertext`, `Auth::Token`, `KeyCache`. Each has a test
  (KSD §7).

---

## 5. OpenBao deployment and custody

Nothing in `ops/openbao/` is applied to the cluster yet. Deployment is card #3424, gated on Erik's
go-ahead (OPS header).

### 5.1 Deployment shape

Sources: KSD §4.1 and `ops/openbao/helm-values.yaml`.

- **Chart and image:** the official `openbao/openbao` Helm chart 0.29.6. The image is
  `openbao/openbao:2.7.0`, pinned by digest.
- **Mode:** standalone, one replica. The injector and the UI are disabled.
- **Storage:** integrated **Raft** storage at `/openbao/data`, on a 10 GiB PVC with an explicit
  `storageClass: hcloud-volumes`. The cluster has two default StorageClasses, and an implicit one
  could land on node-local disk (#3488).
- **No Raft snapshots are configured, deliberately.** A snapshot history of key material would keep
  destroyed keys alive. Replication without that history is #3507's job.
- **Telemetry:** Prometheus, with `unauthenticated_metrics_access = true` on the listener.
- **Audit:** the chart's audit storage is disabled. The audit device is declared in the server
  configuration (§5.8).
- **NetworkPolicy** (`k8s/networkpolicy.yaml`):
  - Ingress to the OpenBao pods on 8200 from any pod in namespace `fau-app`, from pods labelled
    `app.kubernetes.io/name: fau-keys-operator` in the same namespace, and from any pod in namespace
    `monitoring`.
  - Egress to kube-dns on 53, and to **any destination** on 443 and 6443. This exists for
    TokenReview against the API server, and is "to narrow once #3424 knows it".
  - **Discrepancy (minor):** KSD §4.1 says "from the app's pods, the deletion job and monitoring
    only". The policy admits the whole `fau-app` and `monitoring` namespaces.
- **Deletion-job pods:** `runAsNonRoot` (uid 100, gid 1000), seccomp `RuntimeDefault`, a read-only
  root filesystem, all capabilities dropped, and no privilege escalation.

### 5.2 Memory locking and swap

- OpenBao 2.x dropped mlock. 2.7.0 refuses a configuration that sets `disable_mlock`, and the 0.29.6
  chart has no `IPC_LOCK` and sets `SKIP_SETCAP=true` (KSD §4.1).
- The compensating control is **no swap on the nodes**. It is to be verified on the `cx23` nodes
  under #3424, and is not verified yet.
- Core dumps are disabled by OpenBao itself, and the process runs non-root.
- ADR-003 §5a requires the same (no swap, no core dumps) of the backend.

### 5.3 TLS with an internal CA

Source: `k8s/certificates.yaml`.

- **CA:** a cert-manager self-signed bootstrap Issuer issues the CA certificate
  `fau-openbao-internal-ca`: ECDSA P-256, `duration: 87600h` (10 years), `isCA: true`. A CA Issuer
  is built on it, separate from the public ACME issuers.
- **Server certificate:** `openbao-server`: ECDSA P-256, `rotationPolicy: Always`,
  `duration: 2160h` (90 days), `renewBefore: 360h` (15 days), so it renews about every 75 days.
  - SANs are the in-cluster service names plus `localhost` and `127.0.0.1`. The loopback SANs let
    in-pod `bao` and the operator's port-forward validate.
- **Distribution:** the app trusts the CA through `ca.crt` in the server certificate's Secret, to be
  distributed to the app namespace by #3424.
- **Key storage:** both the CA private key and the server private key live as Kubernetes Secrets,
  and therefore in etcd and its snapshots. [Brief author's observation: ADR-003 §5 forbids key
  material in Kubernetes Secrets for data keys. TLS keys are not covered by that rule, but an etcd
  snapshot yields the ability to mint OpenBao server certificates.]

**Certificate reload** (OPS §11):

- OpenBao reads its listener certificate only at start and on SIGHUP. After every renewal, an
  operator must send SIGHUP to the `bao` process by name (`kill -HUP "$(pidof bao)"`).
- Sending it to PID 1 would kill the shell wrapper, restart the pod and **seal** OpenBao.
- A restart is not a remedy, because a restarted OpenBao comes up sealed.
- Until the SIGHUP, the old certificate is served, and clients fail when it expires.
- **This is a manual step every ~75 days, with no alert yet.** The alert on the served
  certificate's expiry belongs to #3442 and does not exist.

### 5.4 Seal, unseal and custody

Sources: KSD §4.2, OPS §2–5, and PD "Key service design with OpenBao agreed" (27 September 2026).

- **Seal type:** Shamir with **a single key share**. `bao operator init -key-shares=1
  -key-threshold=1`, run inside the pod.
- **Unseal key custody:**
  - Erik stores it in Proton Pass as "OpenBao unseal key" (#3481) and enters it with
    `bao operator unseal` after every start.
  - Proton Pass "holds the unseal key only" (KSD K6).
  - It is never on disk in the cluster: not in an image, a Secret, a volume or a config file.
    This inherits the 24 September 2026 decision on the "live root key" (ADR-003 §5).
- **While sealed:**
  - every transit call fails (503), so all encrypted content is unavailable;
  - login, authorization and navigation keep working;
  - alert `KeyServiceSealed` is critical after 2 minutes (§5.9).
  - **Availability therefore depends on one person.** A node reboot or pod reschedule leaves
    content unreadable until Erik unseals (ADR-003 §5, accepted consequence).
- **Root token:**
  - The one printed by `init` is used once for `configure.sh cluster` and then revoked with
    `bao token revoke -self`.
  - When one is needed again, Erik runs `bao operator generate-root -init`, then `generate-root`
    with the unseal key, then `-decode` with the one-time password. He uses the token and revokes it.
  - **No standing root token exists.** The reason given is OpenBao's own guidance: a standing root
    token bypasses every policy.
- **Human-only operations.** `init`, `unseal` and `generate-root` against the real OpenBao are
  Erik's alone, on his own terminal, **never through an AI agent**, because agent transcripts
  persist.
- **Configuration** is applied through a port-forward with an explicit `BAO_CACERT`, and the root
  token is typed by hand (OPS §3).

[Brief author's observation] With a 1-of-1 Shamir split, **the unseal key alone is sufficient to
generate a root token.** So the Proton Pass item is equivalent to full administrative control of
OpenBao, not a lesser credential. Together with a copy of the Raft volume, it is sufficient to
recover every key that existed when that copy was taken.

### 5.5 Kubernetes authentication

Source: `configure.sh cluster`.

- The Kubernetes auth method is enabled, with `kubernetes_host="https://kubernetes.default.svc"`.
  TokenReview uses OpenBao's own service account (`server.authDelegator.enabled: true`).
- Role **`fau-app`**: `bound_service_account_names=fau-app`,
  `bound_service_account_namespaces=fau-app`, `policies=fau-app`, `token_ttl=1h`,
  `token_max_ttl=24h`.
- Role **`fau-keys-operator`**: `bound_service_account_names=fau-keys-operator`,
  `bound_service_account_namespaces=openbao`, `policies=fau-keys-operator`, `token_ttl=15m`,
  `token_max_ttl=1h`.
- The deletion job logs in through `job.sh` (`auth/kubernetes/login role=fau-keys-operator`) and
  execs `shred.sh`.

### 5.6 Policies, exactly

**`fau-app`** (`ops/openbao/policies/fau-app.hcl`):

```hcl
path "transit/keys/+"              { capabilities = ["update"] }   # create a key; never read, list or delete
path "transit/datakey/plaintext/+" { capabilities = ["update"] }
path "transit/datakey/wrapped/+"   { capabilities = ["update"] }
path "transit/encrypt/+"           { capabilities = ["update"] }
path "transit/decrypt/+"           { capabilities = ["update"] }
```

Everything else is denied by default. The policy-matrix test (`backend/crates/keys/tests/openbao.rs`,
`the_policies_allow_exactly_what_the_spec_says`) asserts 403 for the app on all of these:

- `GET transit/keys/<k>` and `LIST transit/keys`;
- `DELETE transit/keys/<k>`;
- `POST transit/keys/<k>/config`, `/rotate` and `/trim`;
- `DELETE /soft-delete` and `POST /soft-delete-restore`;
- `GET transit/export/encryption-key/<k>` and `GET transit/backup/<k>`;
- `POST transit/encrypt/<k>` on a key that does not exist (no auto-create);
- any write to `fau-keys-queue/`.

Two discrepancies with KSD §4.3:

- **Discrepancy:** KSD says the app may use `transit/keys/fau-*` "**create only** (it can create a
  key but not read, change or delete one)". The code grants `update`, because OpenBao 2.7.0 does not
  apply `create` to `transit/keys/:name` (verified in the plan spike).
  - `update` on an existing key name returns that key's metadata without changing it, including
    for a soft-deleted key.
  - So the app can in effect read key metadata by re-posting a create.
  - It cannot change key parameters on an existing key, as far as the spike established.
- **Discrepancy:** KSD writes the allowed paths as `.../fau-*`. The policy uses `+`, meaning any
  single path segment, and does not restrict names to the `fau-` prefix. There is also no tenant
  scoping: the app can create, datakey, encrypt and decrypt under any tenant's keys.

**`fau-keys-operator`** (`ops/openbao/policies/fau-keys-operator.hcl`):

```hcl
path "transit/keys"                      { capabilities = ["list"] }
path "transit/keys/+"                    { capabilities = ["read", "delete"] }  # metadata only
path "transit/keys/+/config"             { capabilities = ["update"] }
path "transit/keys/+/soft-delete"        { capabilities = ["delete"] }
path "transit/keys/+/soft-delete-restore"{ capabilities = ["update"] }
path "fau-keys-queue"                    { capabilities = ["list"] }
path "fau-keys-queue/+"                  { capabilities = ["create", "read", "update", "delete"] }
```

- Tests assert the operator gets 403 on `transit/decrypt`, `transit/datakey/plaintext`,
  `transit/encrypt`, on creating a transit key, and on writing a policy.
- It can read metadata and list keys. It cannot rotate, trim, export or back up keys.
- **"The job that can destroy keys cannot read anything"** (KSD §4.3).

**The `+` versus `*` lesson** (the policy file comment; CLAUDE.md):

- A `*` glob, as in `transit/keys/*`, matches every deeper path.
- It would therefore also grant `transit/keys/<name>/config` (including `deletion_allowed`),
  `/rotate`, `/trim` and `/soft-delete`. This was verified on 27 September 2026.
- Policies use `+`, which matches one segment only. Changing a policy requires updating the policy
  matrix test.

[Brief author's observation] The operator can set `deletion_allowed=true` and hard-delete **any key
immediately, without a prior soft delete or waiting**. The test
`the_operator_can_destroy_and_queue_with_its_own_token` sets `deletion_allowed` before the soft
delete. **The 7-day window is enforced only by `shred.sh`, not by OpenBao policy.**

- A stolen `fau-keys-operator` token, or anyone able to run a pod under that service account in the
  `openbao` namespace, can destroy every FAU's keys at once. It can also backdate the queue.
- The audit-log alerts that would catch this are not live yet (§5.9).
- ADR-003 §7 requires that "compromising the key service must not grant the ability to cancel".
  #3507 owns whether the hard delete moves to a replica side.

### 5.7 Rate-limit quota

- `configure.sh cluster` sets one quota: `sys/quotas/rate-limit/fau-transit path=transit/ rate=50
  interval=1s`. The dev mode uses 1000/s.
- It is a starting value, to be tuned under #3442. It replaces the 26 September design's
  lease-enforced ceiling (KSD §4.3, "Ruling").
- **Discrepancy:** KSD §4.3 says "Rate-limit quotas **on the `fau-app` role's transit paths**". The
  quota is path-based on the whole `transit/` mount and is not role-scoped. It applies to every
  caller, including the deletion job's `transit/keys` calls. It is also one shared bucket for all
  tenants.
- [Brief author's observation] 50 requests per second is about 180,000 unwraps per hour. That is
  far above the whole expected key population of about 1,000 record keys plus documents plus chat
  months. So the quota bounds the speed of a mass decryption only loosely, and the distinct-keys
  alert is the real detector. The quota is also a single cross-tenant denial-of-service point: any
  runaway client makes all content `content_unavailable` for everyone.

### 5.8 Audit device

- The audit device is `audit "file" "stdout"`, declared in the server configuration (`helm-values.yaml`
  and `dev-server.hcl`). OpenBao 2.7.0 refuses to create audit devices through the API ("cannot
  enable audit device via API; use declarative, config-based audit device management").
- It is applied on start and on SIGHUP. Output goes to container stdout and is collected into Loki
  (KSD §4.4).
- **OpenBao HMACs request and response values** in audit entries, so key material, plaintext and
  ciphertext are not logged in the clear.
- Request paths are logged in the clear. They contain the key name, and therefore the tenant UUID
  and document UUID. The audit log is thus a per-tenant, per-document access record. That is by
  design: the mass-decryption alert depends on it.
- [Brief author's observation] The audit trail lives only in stdout, then the node's container log
  files, then Loki. Its integrity and retention depend on the logging pipeline. There is no second
  audit device, and nothing makes the log tamper-evident.

### 5.9 Alerts: live versus commented out

Sources: `ops/openbao/k8s/alerts.yaml`, OPS §9 and KSD §4.4.

| Alert | Source | Severity | Status |
|---|---|---|---|
| `KeyServiceSealed` | `max(vault_core_unsealed) == 0` for 2m | critical | **Live** in the `PrometheusRule` (not yet deployed) |
| `KeyServiceRateLimited` | `increase(vault_quota_rate_limit_violation[15m]) > 0` | warning | **Live** in the `PrometheusRule` (not yet deployed) |
| `KeyServiceManyDistinctKeysDecrypted` | Loki: distinct keys under `transit/decrypt/fau-.*` over 1h `> 50` | critical | **Commented out**, pending #3442 verifying audit field names against a real line |
| `KeyServiceSoftDeleteOrRestore` | Loki: any `transit/keys/.+/(soft-delete\|soft-delete-restore)` in 15m | critical | **Commented out** |
| `KeyServiceShredCritical` | Loki: `shred: CRITICAL` lines from container `shred` in 15m | critical | **Commented out** |
| Chat key deleted before month + 12 months | the deletion job's own check, logged as CRITICAL | critical | Only as a CRITICAL log line and a failed Job (exit 3), which the commented-out rule would catch |
| Served TLS certificate near expiry | — | — | **Not written**; #3442 |
| Hard delete (`DELETE transit/keys/<k>`) | — | — | **No rule.** [Brief author's observation: KSD §4.4 lists "Any soft-delete, soft-delete-restore or key delete" as critical; the drafted Loki rule matches soft-delete and restore only, not a direct hard delete] |

- **Routing:** critical alerts go to Signal via Healthchecks.io, and warnings go to email. Alert
  names carry no member data (PD "Alerting endpoint accepted", 10 September 2026). Alertmanager
  routing is decided but not configured.
- **Consequence:** until #3442 lands, **none of the three alerts for mass decryption, soft
  delete/restore and shred errors exists.** ADR-003 §7 calls these alerts the control that makes a
  silent bulk deletion or bulk decryption impossible.

---

## 6. Deletion and crypto-shredding

### 6.1 The FAU deletion flow (governance, ADR-003 §7a)

Decided 22 September 2026 (PD "Deletion timings", 22 September 2026). Not built.

**The request freezes the FAU immediately.** Billing, invitations and writes stop, and reads
continue so members can export.

**Who must confirm depends on the membership history:**

- An FAU that **only ever had one member** may self-delete.
- Every other FAU needs confirmation from the recovery contact.
  - Where that is a school representative, FAU waits **at least 7 days** for a response. Silence is
    not consent: the FAU stays frozen and pending.
  - If the wait would expire inside the **summer holiday**, the clock restarts at the end of it.
    Only the summer holiday counts (ADR-003 "Closed").
- **Cancellation** of a queued destruction takes **one operator**, a standing risk.

**Only after confirmation** does the operator run `shred.sh fau <tenant>` as a one-off Job (OPS §7).

**Two deletions are distinguished and must not be conflated.** Closing an FAU account is this flow.
An individual's GDPR Article 17 erasure has a one-month statutory clock and concerns that person's
own data, not the FAU's shared documents. It is to be specified on #3426 and must not inherit these
waits (ADR-003 §7a).

### 6.2 The cryptographic step: soft delete, then hard delete after 7 days

**Why 7 days** (ADR-003 §7; PD, 22 September 2026). The window started as 12 hours, was raised to
"at least 48 hours, probably 72+", and settled at **7 days**. 72 hours does not survive a Norwegian
Easter: Skjærtorsdag through 2. påskedag is five days.

**With OpenBao** (KSD §5; ADR-003 §7 amended 27 September 2026):

- "Queued" means **transit soft delete**. Every key of the FAU stops working at once, and can be
  restored during the window.
- After 7 days the deletion job sets `deletion_allowed=true` and hard-deletes each key.
- In #3506 the same CronJob runs the hard delete. #3507 decides whether it must move to the replica
  side, because ADR-003 §7 wants compromising the key service not to grant the ability to cancel.

**The queue.** OpenBao records no soft-delete timestamp, so the job keeps its own queue in an
operator-only KV v1 mount, `fau-keys-queue/` (plan ruling 1). The app is denied the mount, and a
test checks this.

- One entry per key name, holding `soft_deleted_at` (Unix seconds, UTC) and `reason`.
- `reason` takes one of three values: `fau`, `document` or `chat-expire`. The proposed archive
  design adds `archive` and `export-expire`.

**`shred.sh` behaviour** (`ops/openbao/shred.sh`, run by `job.sh` under `fau-keys-operator`, with
`WINDOW = 7*24*3600`):

- **`fau <tenant>`**
  - Validates that the tenant is a UUID, then checks reachability with `bao token lookup`.
  - Lists `transit/keys` and soft-deletes every name starting `fau-<tenant>-`, queued with
    `reason=fau`.
  - Exits 4 if nothing matched.
- **`document <tenant> <document>`:** soft-deletes `fau-<t>-doc-<d>`, queued with
  `reason=document`.
- **`chat-expire`** (CronJob `fau-keys-chat-expire`, `41 3 2 * *`, the 2nd of every month at 03:41
  UTC):
  - For every `fau-*-chat-YYYY-MM` key whose month index is **13 or more** behind the current UTC
    month index, it soft-deletes and queues the key with `reason=chat-expire`.
  - "Young" means "month ended less than 12 months ago". It matches `ChatMonth::index()` in Rust.
- **`restore <key>`:** `soft-delete-restore`, then deletes the queue entry. It logs
  `shred: CRITICAL restored <key>` and exits 0.
- **`finalize`** (CronJob `fau-keys-finalize`, `17 3 * * *`, daily at 03:17 UTC). For each queue
  entry:
  1. Skip it if the entry vanished, or if it is malformed (missing or non-numeric
     `soft_deleted_at`), with a CRITICAL log line.
  2. Skip it if younger than 7 days.
  3. Read the key's `soft_deleted` flag. If the key is not soft-deleted (restored out of band, or
     already gone), log CRITICAL and unqueue it without destroying anything.
  4. If it is a chat key whose `reason` is not `fau` and whose month is still young, **refuse**,
     log CRITICAL and set exit 3.
  5. Otherwise, in order: `config deletion_allowed=true`, `DELETE transit/keys/<k>`, delete the
     queue entry.
- **Soft delete is idempotent.**
  - An already soft-deleted and queued key is skipped, and its original `soft_deleted_at` is kept,
    so the clock is not re-armed.
  - A key that is soft-deleted but **not** queued is re-queued with the current time, and logged
    CRITICAL. Its window starts again.
  - Soft delete is done before the queue write, so a crash leaves a key inert and restorable rather
    than active-but-queued.
- **Error discipline:**
  - Nothing is treated as "empty" or "gone" unless OpenBao said so explicitly: an empty list is
    exit 2 with `{}`, and a read shows "No value found at".
  - A read that fails and then succeeds on retry is distrusted and fatal.
- **Job settings:** `concurrencyPolicy: Forbid`, `backoffLimit: 0`, `restartPolicy: Never` and
  `activeDeadlineSeconds: 600`. A non-zero exit is a failed Job.

**`shred.sh` exit codes** (OPS §7). Every non-zero code except 2 logs a `shred: CRITICAL` line.

| Exit | Meaning |
|---|---|
| 0 | Done. Keys already soft-deleted and queued, or gone, are skipped |
| 1 | OpenBao error: unreachable, sealed, token rejected, permission denied, or a failed read or write. Re-running is safe |
| 2 | Bad input (usage) |
| 3 | `finalize` refused a young chat month not queued with `reason=fau` |
| 4 | `fau <tenant>` matched no keys |

**Discrepancy (doc reference):** the `shred.sh` header cites the exit codes at
"docs/key-service-operations.md §4". They are in §7.

**After a transit key is destroyed,** its `wrapped_keys` rows and ciphertext are useless. The
application's own purge deletes them as tidying, which is not the security property (KSD §5). No
such purge exists yet.

### 6.3 Document purge (ADR-003 §7b; SDA §7)

- Purging a document destroys `fau-<t>-doc-<d>` through the same soft-delete, 7-day and hard-delete
  path, with `reason=document`. This makes the document's step batches, checkpoints, title and
  filenames unrecoverable everywhere, including in backups, "with nothing to find and no cleaner to
  run".
- **In-document redaction is a different mechanism.** Removing one sentence from history while the
  document survives means rewriting the step log from a checkpoint. Step-log compaction is therefore
  a privacy mechanism, not only a performance one (ADR-003 §7b).
- There is no age guard on document keys.

### 6.4 Chat expiry (GDC §5.5; KSD §5)

- Chat bodies use a per-FAU monthly key. The key is destroyed once the month ended 12 months ago,
  which makes backup copies unreadable too.
- **Routine expiry is expected and silent.** Any attempt to destroy a younger chat key is critical.
- **Effective timing from the code:**
  - `chat-expire` soft-deletes on the 2nd of the month after the 12-month mark, and `finalize`
    hard-deletes 7 days later.
  - A key for month M is therefore destroyed about 12 months, 2 days and 7 days after the end of M.
  - A message posted on the first day of M lives about 13 months and 9 days.
- **Discrepancy:** GDC §5.5 says "a **daily** job deletes messages older than 12 months and
  destroys that month's key", and that the chat key is "wrapped by the FAU's KEK". The code has a
  monthly soft delete plus a daily 7-day finalize, and there is no KEK.
- **Discrepancy [Brief author's observation]:** GDC §5.5 tells Article 17 requesters that "backup
  copies become unreadable **within 12 months**". Measured from posting, the bound is closer to 13
  months and 9 days.
- The shell's "young" test uses UTC months, while chat months are defined in Europe/Oslo. The skew
  is a matter of hours at month boundaries.

### 6.5 What crypto-shredding does not reach

These items are named in the documents:

- **Published output.** Published HTML and PDF are a released derivative, outside the encryption
  boundary and not encrypted under the document key. Destroying the key leaves them online.
  - Publication needs its own deletion path, and its bucket must permit real deletes. It must not
    carry the permanent-deletion-denied policy used on the Terraform-state bucket (ADR-003 §7b;
    SDA §9).
  - ARC §7 adds that the destroy step must call that path, "or published minutes would outlive the
    FAU indefinitely".
- **Plaintext exports downloaded by members.** Once downloaded, they are out of reach. Server-side
  exports are designed to be encrypted under their own key and shredded after 90 days (ARC §6).
- **Plaintext metadata** in database backups, including poll votes. These outlive a purge until the
  backups age out, and "the privacy notice states the backup retention period" (GDC §7.3).
- **Items the documents do not name** [Brief author's observation]:
  - ICS files downloaded by members, which carry full event details (GDC §6.4);
  - calendar subscriptions already fetched by Google or Apple (redacted, §7);
  - email content sent before deletion (recovery notifications carry ids only);
  - browser caches;
  - the OpenBao audit log, which holds key names, therefore tenant and document ids, and timestamps;
  - the application log;
  - quarantined original uploads, whose retention is deferred to #3426 (ING);
  - anything the backend held in memory while the key was live.
- **The two inbound messages** are shredded with `fau-<tenant>-messages`, but OpenBao processed
  their plaintext.

### 6.6 Why database backups become unreadable, and the replica

**Backups.** Database and object-storage backups contain only ciphertext and wrapped data keys. The
wrapping transit key exists only in OpenBao's Raft store. OpenBao's store has no snapshot history
by configuration, and it is never in a Kubernetes Secret or etcd (ADR-003 §5, §7; KSD §3.2).

**The claim, as worded** (ADR-003 §7): destroying the key "makes its content unrecoverable, in the
live system and in every database backup, **once the queued destruction has completed**. Until then
it is recoverable, and the recovery is itself audited." This is to be proven by the #3425 restore
rehearsal, including a negative control.

**The replica (#3507, not started)** (ADR-003 §7; KSD §1; OPS §8). Erik's requirement is that
ransomware reaching the key service must not destroy every FAU permanently. The designed shape:

- one **live** replica of individually addressable key records, in a store with real deletes, and
  **no generational snapshot history**;
- encrypted under a separate **backup root key** kept off the cluster (Proton Pass, per ADR-003
  "Closed");
- **different credentials** from the key service;
- the queued hard delete executed **from the replica side**.

KSD rescopes #3507 to "replicating OpenBao's key state off the cluster without a snapshot history
of keys; the 7-day hard delete executed from the replica side; the #3425 restore rehearsal". It is
a **hard gate before any real FAU data** (pilot #3432).

**Until #3507 exists, losing OpenBao's volume loses every key and therefore all content,** with no
recovery path. The documents call this acceptable only while there is no real data (KSD §1; OPS §8).

[Brief author's observation] The claim "no snapshot history" refers to Raft snapshot *backups*.
OpenBao's integrated storage still does the following:

- it takes **internal Raft snapshots** for log compaction inside its data directory;
- it keeps a Raft log;
- it uses BoltDB, which does not overwrite freed pages.

So a hard-deleted key's encrypted record may persist on the volume for some time. The volume's
contents are barrier-encrypted, but the barrier keyring is protected by the unseal key, which is
never rotated and is held indefinitely. A copy of the volume taken at any time, plus the unseal
key, may therefore recover a "destroyed" key. Hetzner volume snapshots, if anyone ever takes one,
would do the same. The 26 September hand-written design had specified SQLite with `secure_delete`
for exactly this reason (PD "Key service first", 26 September 2026).

---

## 7. What is outside the encryption boundary

### 7.1 Encrypted

Sources: ADR-003 §6, SDA §7 and GDC.

- **Document content:** bodies (step batches and checkpoints), document titles, original filenames,
  uploaded object bytes, comment bodies and quoted anchor text.
- **Titles and filenames are encrypted deliberately.** "Klage på lærer Hansen" or
  `bekymringsmelding-elev.pdf` discloses as much as the file it names.
- **Group and directory data:** group names, and member `display_name` and `contact_email` (per
  membership, under the record key).
- **Events:** title, location and description.
- **Polls:** title and description.
- **Chat:** message bodies, under the monthly chat keys.
- **Inbound messages:** access-request and invitation messages, by transit directly.

### 7.2 Plaintext, because the product must query it

Sources: ADR-003 §6, SDA §7 and GDC.

- **People:** member **login emails**, names in the sense of account-level identifiers, and the
  access-request sender's email.
  - [Brief author's note] ADR-003 §6 lists "member emails and names" as plaintext. GDC §4.1 later
    encrypted per-membership display names, and Hanko holds no names. The login email is plaintext.
- **Roles and dates:** roles and their date ranges, and membership history.
- **Names of institutions:** FAU and school names. School data comes from the national register.
- **Identifiers and times:** all identifiers (UUIDv7), sizes, hashes, content types, timestamps and
  audit event types.
- **Document metadata:** document `type`, version numbers, author identifiers, task rows'
  scheduling metadata, comment thread structure and resolution state.
- **Groups:** group membership, visibility, and unit or cohort bindings.
- **Chat metadata:** thread structure, message `seq`, author, created and edited times, and read
  markers.
- **Events:** times, kind and revision.
- **Poll votes:** options and votes, as "an answer enum next to a membership id" (GDC §7.3).
- **UUIDv7 leaks creation time** to anyone holding an id. This was accepted on 10 September 2026:
  "we can live with leaking creation times", bounded by ids reaching only people who can open the
  resource (PD "ADR-002 accepted").
- **Key names** in OpenBao and its audit log embed tenant and document UUIDs.

### 7.3 By-design exits and in-session processing

- **Published pages** (SDA §9; not built).
  - The publishing client renders HTML and PDF. The server sanitises with an allowlist and does
    not trust the client.
  - The output is stored and served static from a **separate origin**, with no JavaScript and a
    strict CSP.
  - It is outside the boundary and unaffected by shredding.
- **Calendar subscription feed** (GDC §6.3).
  - One feed per membership at `/cal/<token>.ics`. The token is 256 random bits and stored hashed.
    It is revocable and dies with the membership.
  - The feed carries only a generic localised `SUMMARY` ("FAU-møte", "FAU-arrangement",
    "FAU-frist"), start and end, a cancelled status, `UID`, `SEQUENCE`, a URL back into the app and
    `VTIMEZONE`.
  - **Never in the feed:** title, location, description, FAU name or school name.
  - **The feed handler has no access to the key service.** No decryption happens outside a session.
  - The token path segment must be redacted in access logs, and the feed is rate-limited per token.
  - A single-event `.ics` download inside the session carries full details, with a different `UID`.
- **Search** is filename-only in the MVP. Filenames are decrypted inside the authorised session and
  filtered there, with no index. Nothing leaves the boundary to make search work (ADR-003 §6).
- **Change notifications** (SSE, per FAU) carry identifier and revision only, so notifying an idle
  viewer costs no decryption (ADR-003 §5a).
- **Email** never carries message plaintext. Admin notifications carry ids only (ADR-003 §6).
- **Malware scanning and conversion** of uploads run in-house (ClamAV; `mammoth` for DOCX). They
  run on an ephemeral worker with no database credentials, reading only from a quarantine bucket
  and writing only results, and conversion runs with no network egress (ING).
  - [Brief author's observation] The ingest documents do not say whether the quarantine bucket holds
    plaintext or ciphertext. If ciphertext, the documents do not say how the worker obtains a key.
    ING also still refers to the superseded Lexical editor. This boundary is unspecified.
- **Encrypted uploads are refused** by the ingest pipeline, because they cannot be scanned (ING).

---

## 8. Status

| Item | Status |
|---|---|
| ADR-003 (identity, encryption, deletion, recovery contact) | Accepted 22 September 2026 (PD "Localisation settled, and ADR-003 accepted"). Amended 27 September 2026 for OpenBao |
| KSD (OpenBao design) | Agreed 27 September 2026 |
| `fau-crypto` crate (envelope, AAD, `Unit`, `WrappedKey`, `MessageCiphertext`, `DataKey`) | **Built, merged on `main`** (#3506, PR #4) |
| `fau-keys` crate (transit client, lazy Kubernetes login, `KeyCache`, `data_key` flow, error mapping) | **Built, merged.** Tested against a dev OpenBao |
| Migration 0005 (`wrapped_keys`; message ciphertext columns ≤ 4096 bytes) | **Built, merged** |
| Persistence for access-request and invitation messages (accepts `MessageCiphertext` only) | **Built, merged** |
| Edge mapping `KeyError` → `content_unavailable` and others | **Built, merged** |
| Wiring of `KeyCache` and `Keys` into HTTP handlers, session lifecycle and the idle sweep | **Not built** [Brief author's observation from the code] |
| `ops/openbao/` (Helm values, policies, `configure.sh`, `shred.sh`, `job.sh`, CronJobs, NetworkPolicy, certificates, `PrometheusRule`) | **Written and merged. Not deployed**: #3424 |
| Loki audit alerts, certificate-expiry alert, Alertmanager routing | **Not built**: #3442 |
| Documents with per-document keys, publication, uploads encrypted to S3 | Designed (SDA, #3419). **Not built** |
| Groups, directory, chat with epoch keys, calendar, polls | Designed (GDC, accepted 26 September 2026). #3501 started 27 September on branch `groups-3501`, with no encrypted fields yet |
| Archive and lock lifecycle (`archive`, `restore-fau`, `export-expire`, 365-day window) | **Proposed**: ARC, #3509, awaiting Erik's review |
| Key replica, replica-side hard delete, restore rehearsal | **Not started**: #3507, #3425. Hard gate before real data |
| Deployment of OpenBao to the cluster | **Not started**: #3424 |
| GDPR Article 17 erasure flow | **Not specified**: #3426 |
| Administrative MFA guard (endpoint list, freshness threshold) | **Open**: #3414 |

**Discrepancy (ADR status):** ADR-003's header still reads "Status: proposed". PD records it as
accepted on 22 September 2026.

---

## 9. Known gaps, accepted risks and open questions

### 9.1 Accepted risks stated in the documents

1. **Root on a node defeats everything** (ADR-003 trust model).
2. **A compromised backend decrypts any FAU.** It is bounded only by a 50/s cross-tenant quota and
   by an alert that is not yet live (KSD K3; §5.7, §5.9).
3. **One person** holds the unseal key, is the only one who can unseal, sits in the recovery seat
   for every FAU without a confirmed school representative, and alone can cancel a destruction
   (ADR-003 "Standing risks").
4. **Availability.** A restart seals OpenBao until that person unseals it (ADR-003 §5).
5. **Until #3507, volume loss is total** (KSD §1; OPS §8).
6. **The MFA gate is not per-action step-up** (ADR-003 §4, accepted on #3414).
7. **A malicious recovery contact** can seat someone, and notifications make the window short but
   not zero (ADR-003 §8).
8. **UUIDv7 creation-time leak** (PD, 10 September 2026).
9. **Published output is outside the boundary** and needs its own deletion path (ADR-003 §7b).
10. **Losses accepted in moving to OpenBao:** the lease-enforced concurrency ceiling became rate
    limits plus alerting, and the per-FAU KEK hierarchy became flat keys deleted by prefix (PD, 27
    September 2026).
11. **HPKE sealing withdrawn.** OpenBao sees the plaintext of the two inbound messages (KSD §3.3).
12. **Hanko's AWS subprocessor** was accepted (ADR-003 §1). Hanko's ISO 27001 is "in progress", and
    nothing is claimed on its behalf.

### 9.2 Known gaps in the documents

- **Single unseal share** (`-key-shares=1 -key-threshold=1`), by decision (KSD §4.2).
- **Manual SIGHUP after every TLS renewal** (~75 days). There is no expiry alert yet, and a mistaken
  restart seals OpenBao (OPS §11).
- **Rate-limit scope:** one path-wide quota, not role-scoped, and not tuned (§5.7).
- **Audit alerts** for mass decryption, soft delete/restore and shred errors are **commented out**
  pending field-name verification (OPS §9).
- **No swap** on the nodes is a requirement that is not yet verified (#3424).
- **NetworkPolicy egress** allows 443 and 6443 to anywhere until narrowed (#3424).
- **The replica's window** must follow the queue reason: 365 days for `archive` in the proposed
  design (ARC §9).
- **The legal basis for archive retention** must be in the terms and the DPA before the archive
  ships (ARC §9).
- **Retention of lapsed members' emails** for recovery notification: "24 months proposed", to go
  into the DPA (ADR-003 §10). See the inconsistency below.
- **The Article 17 flow** is unspecified (#3426).
- **Administrative MFA:** the sensitive-endpoint list and the freshness threshold (30–60 minutes)
  are open (#3414). Whether a passkey satisfies the second-factor check is also open.

### 9.3 Inconsistencies between documents (beyond those flagged inline)

- **Stale pre-OpenBao hierarchy in the documents.** SDA §7 ("document key wrapped under the FAU
  key-encryption key"), GDC §5.5 and §8 ("wrapped by the FAU's KEK") and ADR-003 §5 points 1–4
  still describe a KEK hierarchy with keys held "only in the key service's datastore".
  - ADR-003 §5 carries an amendment note saying so.
  - ADR-003 §6 still says content is "encrypted under the FAU data key", although documents now
    have per-document keys.
- **The "several wraps per FAU key" hook** for future member-held keys (ADR-003 §5, 24 September
  2026) has no counterpart in the flat OpenBao design. It is unclear how the post-MVP direction
  would now be built.
- **Retention periods conflict inside ADR-003.** §6a lapses an account 3 months after its last
  membership ends. §10 retains lapsed members' email addresses for recovery notification for a
  proposed 24 months.
- **Passkeys in the MVP.** ADR-003 §4a calls it an open item, while ADR-003 "Closed" says "No.
  Passcode only". `CLAUDE.md` calls it open.
- **ADR-003's testing table** still expects a key-service-enforced ceiling, "no endpoint returns
  more than one key", and per-KEK semantics. Several rows no longer map onto OpenBao.

### 9.4 Brief author's observations (not in the documents)

1. **The 7-day window is script-enforced, not policy-enforced.** `fau-keys-operator` can set
   `deletion_allowed` and hard-delete any key at once, and can write the queue. A compromise of
   that service account, or of anyone who can create a pod under it in `openbao`, is an immediate,
   unalertable (today) destruction of every FAU.
2. **The unseal key is root-equivalent** under 1-of-1 Shamir, because `generate-root` needs only it.
3. **Remnants on the OpenBao volume** (internal Raft snapshots, the Raft log, BoltDB free pages)
   may keep "destroyed" keys recoverable with the unseal key (§6.6).
4. **Keys created after a prefix sweep escape it.** `shred.sh fau` soft-deletes the keys that exist
   at that moment. A key the app creates afterwards (for example a new chat month or document) is
   never queued. Only the application-level freeze stops new keys being created.
   - After a hard delete, the app will also re-create a key of the same name on next use, because
     `ensure_key` is called before every data-key or message encryption.
   - In the proposed archive flow, ARC §4 requires evicting the tenant's keys from every instance's
     `KeyCache`. No such method exists: only per-session and per-document release.
5. **AAD does not bind a revision**, so cell-level rollback is possible for an attacker with
   database write access. The envelope version byte is also not in the AAD (§4.4).
6. **No tenant scoping in the app policy.** `+` admits any key name (§5.6).
7. **The hard delete has no alert rule,** only soft-delete and restore (§5.9).
8. **The TLS CA private key lives in a Kubernetes Secret**, and therefore in etcd snapshots
   (§5.3).
9. **The audit log's integrity** depends on the stdout-to-Loki pipeline (§5.8).
10. **The ingest boundary is unspecified:** whether plaintext reaches the quarantine bucket (§7.3).
11. **Soft-deleted content returns 404 `not_found`,** not a distinct state. During an archive lock
    or a deletion window, users see content as missing (`client.rs` `map_err`).
12. **`transit/datakey/wrapped` is allowed but unused.** The app only ever calls
    `datakey/plaintext`.

---

## 10. Questions for the reviewer

1. **AAD sufficiency.** The local AAD is `"fau-aad-v1" ‖ tenant ‖ len‖table ‖ len‖column ‖ row_id`.
   Should it also bind a revision or sequence, to stop replaying an older ciphertext into the same
   cell, and the envelope version byte? Is binding the unit or key identity worth it, even though
   one data key per unit makes a cross-unit swap fail anyway?
2. **Random 192-bit nonces with XChaCha20-Poly1305** under a long-lived record data key per FAU,
   which is never rotated. Is the lack of any rotation story a concern, and is a key id in the
   envelope worth adding now while the version byte leaves room?
3. **Envelope versus direct transit.** Is AES-256-GCM96 in transit, wrapping a handful of data keys
   and short messages per transit key, adequately justified? Is it acceptable that the inbound
   messages' plaintext passes through OpenBao, compared with the withdrawn HPKE design?
4. **The session key-cache window.** Is "once per session per unit, idle timeout of tens of
   minutes, refreshed by real activity" the right trade-off against per-operation unwrap, given that
   a compromised backend holds plaintext anyway? Is the per-session rather than per-instance cache
   the right granularity for the audit signal?
5. **Transit key granularity.** One transit key per document and per chat month means many
   thousands of transit keys. Does OpenBao transit behave acceptably at that count, for listing
   (`shred.sh` lists all keys to match a prefix), memory and Raft size? Would transit's derived keys
   with a context be a better structure, or do they defeat per-unit shredding?
6. **Crypto-shredding on OpenBao's Raft storage.** Given internal Raft snapshots, the Raft log and
   BoltDB page reuse, is a hard-deleted transit key actually unrecoverable from a later or earlier
   copy of the volume plus the (never-rotated) unseal key? Should the barrier keyring or the unseal
   key be rotated as part of shredding, or is a different backend needed?
7. **Where the 7-day window is enforced.** The window lives in a shell script, while the operator
   policy can hard-delete immediately. What is the best achievable enforcement within OpenBao
   (policy parameter constraints on `deletion_allowed`, a control group, a separate approle only the
   replica side holds)? Or must it move to the replica, as #3507 contemplates?
8. **Soft-delete window versus ransomware.** Seven days of soft delete protects against accidental
   or unauthorised destruction only if an attacker cannot also hard-delete, tamper with the queue or
   suppress alerts. Is the design sound once the replica exists? What properties must the replica
   have (independence of credentials, replica-side deletion, backup root key custody)?
9. **Operator and app separation.** Is the split between `fau-app` (create, datakey, encrypt,
   decrypt, no tenant scoping) and `fau-keys-operator` (destroy, never read) meaningful when both
   run in the same cluster, bound by Kubernetes service accounts, and anyone with cluster-admin can
   impersonate either? Is per-tenant policy scoping worth its complexity?
10. **Custody.** A single Shamir share in one person's password manager is root-equivalent. What
    would you recommend for a one-person operation that must also stay unsealable: 2-of-3 shares, an
    auto-unseal with an EU KMS or HSM, or a transit auto-unseal from a second OpenBao? What would
    each change in the "database and key volume both stolen" case?
11. **Audit sufficiency.** Is an HMACed file audit device to stdout, collected by Loki with alert
    rules on request paths, adequate as the primary mass-decryption detector? Should there be a
    second audit device, tamper-evidence, or an alert on direct hard deletes? Is a threshold of 50
    distinct keys per hour meaningful at about 1,000 FAU-er?
12. **Rate limiting.** Is one path-wide 50 requests/second quota on `transit/` the right shape, or
    should quotas be per role, per tenant key prefix or per operation? How would you bound a
    compromised backend's decryption rate without making the quota a cross-tenant outage lever?
13. **Wrapped keys in the application database.** Is the argument that a wrapped key is safe in
    Postgres because the wrapping key exists at shredding granularity complete? Are there attacks
    via a database attacker who can insert or swap `wrapped_keys` rows, for example substituting a
    wrapped key they generated under the app role for a new unit?
14. **The boundary's metadata leakage.** Given the plaintext list in §7.2, including login emails,
    roles, school names, UUIDv7 timestamps, key names in audit, vote rows and thread structure,
    what does a stolen database reveal about a school's parents' council that the trust statement
    should acknowledge?
15. **TLS and trust distribution.** Is a 10-year self-signed internal CA with its key in a
    Kubernetes Secret acceptable for the backend-to-OpenBao channel? Would you require mTLS in
    addition to Kubernetes service-account authentication, as the 26 September design had?
