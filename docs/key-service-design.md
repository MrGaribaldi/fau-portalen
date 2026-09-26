# Key service: design

Status: **design agreed in chat, 26 September 2026**, section by section, with Erik. He asked for
the implementation plan to start the same evening. This document implements ADR-003 decisions 5,
5a, 6, 7 and 7b (docs/identity-and-encryption.md). It decides what ADR-003 left to
implementation, and resolves one inconsistency in ADR-003. Rulings the agent made without an
explicit answer are marked **Ruling** so they can be challenged.

## 1. Why now, and what is phased

Every feature designed on #3500 stores encrypted fields: group names, member names, chat, event
details. The documents migration (#3419) does too. No key service exists, and no card for one
existed. Erik chose on 26 September to build the key service **before** #3501, rather than
building features against a stand-in cipher.

It is split in two:

| Card | Scope | Gate |
|---|---|---|
| **Key service** | Everything in sections 2–7 | Blocks #3501, #3502, #3503, #3504 and #3419 |
| **Key replica** | The ransomware replica on separate infrastructure, with its own credentials; the backup root key off the cluster; the 7-day destruction queue executed from the replica side; the #3425 restore rehearsal | **A hard gate before any real FAU data**: #3432 (pilot selection) depends on it |

Until the replica exists, losing the key service's volume loses every key and therefore all
content. That is acceptable only while there is no real data.

## 2. Decisions

| # | Question | Decision |
|---|---|---|
| K1 | Build order | Key service first, before #3501 |
| K2 | Replica | Phased: the queue is modelled in card 1; the replica is card 2, which gates the pilot |
| K3 | How the key service trusts the backend | **A service credential plus limits.** mTLS and a NetworkPolicy; the backend states account, session and FAU. The key service knows nothing about memberships and does not call Hanko. ADR-003 5a names rate and ceiling as the controls that matter |
| K4 | Shape and store | **A separate binary with SQLite on its own volume.** Postgres was rejected, because a shared cluster puts keys in the app's backups and a dedicated one brings WAL and PITR habits. An in-process library was rejected, because the backend must never hold a KEK |

## 3. Key classes and operations

### 3.1 Hierarchy

- The root key, in memory only.
  - One KEK per FAU, which never leaves the service.
    - Every other key below.

Every wrap is XChaCha20-Poly1305, with the key's id and class as associated data, so a wrapped
key cannot be moved onto another row.

| Class | Per | Protects | Destroyed when |
|---|---|---|---|
| **KEK** | FAU | the FAU's other keys | the FAU is deleted (ADR-003 decision 7's queue) |
| **Record key** | FAU | small fields outside documents: group names, member display names and contact emails, event title/location/description, poll title/description | with the KEK |
| **Document key** | document | the document's step batches, checkpoints, title, filenames and bytes | the document is purged (decision 7b) |
| **Chat epoch key** | FAU × month (Europe/Oslo) | chat bodies from that month | 12 months after the month ends |
| **Sealing key pair** | FAU | inbound text from non-members (access-request messages) | with the KEK |
| **Invitation key** | invitation | the invitation message | with the KEK. **Ruling:** earlier destruction when an invitation settles is deferred; the message is small and short-lived, and adding it later needs no schema change |

**The record key resolves ADR-003's inconsistency.** Decision 5 moved to per-document keys, but
5a still spoke of "an FAU data key held once per session". The record key is that key: held once
per session per FAU, for everything that is not a document. Document keys stay per document,
which is what lets one document be purged. ADR-003 5a is amended to say so.

Every wrapped key has room for several wraps: a `wraps(key_id, kind, ...)` table where `kind` is
`root` (for a KEK) or `kek` today, and later one wrap per member device or an offline escrow
wrap. This is the requirement Erik set on 24 September.

**Content is encrypted in the backend; the key service never sees it.** It hands out data keys
and nothing else. Sealing needs only the public key, which is served freely.

### 3.2 Operations

There are no list, bulk or export endpoints, and nothing returns a KEK or the root key. A test
pins the route list.

**Lifecycle:**

| Operation | Returns |
|---|---|
| `create_fau(fau)` | Creates the KEK, the record key and the sealing pair; returns the public key only. Idempotent for a FAU that already exists |
| `create_document_key(fau, doc)` | The new key, under a lease |
| `create_invitation_key(fau, invitation)` | The new key, under a lease |

**Unwrap:** one key per call, each for a stated `(account, session)`, each returning a lease.

| Operation | Notes |
|---|---|
| `record_key(fau)` | |
| `document_key(fau, doc)` | |
| `chat_epoch(fau, month)` | The current month (Europe/Oslo) is created lazily; past months unwrap only if they still exist; future months are refused |
| `sealing_private(fau)` | For the admin approval screen |
| `invitation_key(fau, invitation)` | |

**Destroy:**

| Operation | Notes |
|---|---|
| `destroy_document_key(fau, doc)` | Removed at once |
| `destroy_chat_epoch(fau, month)` | **Refused, with a critical alert, unless the month ended at least 12 months ago** |
| `destroy_fau(fau)` | The live KEK is removed at once and a `destruction_queue` row is written. The replica side that executes it is card 2 |

**Public:** `public_key(fau)`, with no session.

### 3.3 Leases and the concurrency ceiling

- Every unwrap and every create returns a lease: an id plus an idle timeout of 30 minutes by
  default.
- The backend calls `renew(lease)` on real user activity and `release(lease)` on logout, session
  end or idle.
- The key service counts live leases per backend instance (the mTLS client identity plus a stated
  instance id). **It refuses a key for a new distinct FAU above the ceiling**, and alerts when the
  number of new distinct FAU keys per hour crosses a threshold.
- Leases are what make the ceiling enforceable. Without them, the key service cannot know what the
  backend still holds, and the ceiling would be a log line.
- Rate limits apply per session and per backend instance.

## 4. Storage, sealing and unsealing

### 4.1 Store

SQLite on a dedicated volume, accessed through sqlx's SQLite driver, so the workspace keeps one
database library. The connection sets `secure_delete = ON`, `journal_mode = DELETE` and
`synchronous = FULL`, with one writer.

| Table | Contents |
|---|---|
| `keys` | `id`, `fau_id`, `class`, `subject` (document id, month `YYYY-MM`, or invitation id; null for per-FAU classes), `created_at`; unique `(fau_id, class, subject)` |
| `wraps` | `key_id`, `kind`, `nonce`, `ciphertext` |
| `public_keys` | `fau_id`, `public_key`. Not secret; stored so it can be served without unwrapping |
| `destruction_queue` | `key_id`, `fau_id`, `enqueued_at`, `due_at` (+7 days), `cancelled_at` |
| `canary` | One value encrypted under the root key, to detect a wrong root key |

- **Destroying a key deletes its rows.** No soft delete, and no tombstone holding material. The
  audit log records the destruction.
- `secure_delete` overwrites freed pages, and the rollback journal (not WAL) means no stale copy
  lingers in a WAL file. **The volume is never snapshotted or backed up**; card 2's replica is the
  only other copy.
- Every stored key is wrapped under the in-memory root key, so the file alone yields nothing.

### 4.2 Logs

Every call logs one JSON line to stdout, collected into Loki like the app's logs: operation, key
class, FAU, subject, account, session, backend instance, lease and outcome. **No key material, no
nonce, and no ciphertext ever appears in a log line or a metric**; a test asserts this.

### 4.3 First start

`fau-keys init` generates the 32-byte root key and prints it **once**, as base64url plus a
4-character checksum that catches paste errors. It then writes the canary. It refuses to run
against a store that already has a canary.

**Erik runs `init` and `unseal` on his own terminal, never through the agent.** Anything printed
in an agent session ends up in a transcript (the 10 September S3-key leak).

### 4.4 Sealed state

- The pod starts sealed.
- `/health/live` answers 200. `/health/ready` reports `sealed`, but the pod stays in the Service
  endpoints, so the backend gets a clear `503 sealed` instead of a connection failure.
- Every key operation returns `sealed`, and the metric `key_service_sealed` is 1.
- As ADR-003 says, login and authorization keep working in the backend; only content is
  unreadable.

### 4.5 Unsealing

- The unseal listener binds **`127.0.0.1`** inside the pod, so it is reachable only through
  `kubectl port-forward`, never over the cluster network.
- `fau-keys unseal` reads the key without echoing it.
- The service checks the canary:
  - a wrong key leaves the service sealed, and counts `key_service_unseal_failures`;
  - a correct key clears the sealed flag.

### 4.6 Process hardening

- The root key is held in `Zeroizing<[u8; 32]>`.
- At start the process sets `PR_SET_DUMPABLE=0` and `RLIMIT_CORE=0`.
- The pod runs non-root, with a read-only root filesystem and no service-account token.
- Swap is off on the nodes, as k3s requires.

### 4.7 Deferred

Rotating the root key (rewrapping every KEK) is deferred. The hierarchy makes it possible without
touching content.

## 5. The backend's side

### 5.1 `crates/crypto`

Used by the backend, not by the key service's storage code.

- **`Ciphertext`**: an opaque newtype. Persistence functions for encrypted columns accept only
  `Ciphertext` and store `bytea`, so plaintext has no path into those columns.
- **Envelope:** `version (1 byte) ‖ key_id (16 bytes) ‖ nonce (24 bytes) ‖ ciphertext`. Every value
  names the key it needs, which also allows a record key to be rotated later.
- **Associated data:** `tenant_id ‖ table ‖ column ‖ row id`. Ciphertext moved to another row,
  column or FAU fails to decrypt.
- **`FieldCipher`**: `encrypt(key, aad, &str) -> Ciphertext` and
  `decrypt(key, aad, &Ciphertext) -> Zeroizing<String>`.
- **Sealing:** HPKE (RFC 9180) with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256 and ChaCha20-Poly1305
  in base mode, via the `hpke` crate.

### 5.2 `KeyClient`

- HTTP/JSON over **mTLS**, with axum on the server and reqwest on the client.
- In production, certificates come from a cert-manager **internal CA Issuer**, separate from the
  ACME issuers.
- In development, `fau-keys dev-certs` writes certificates to a gitignored directory, so
  development and tests also use mTLS. There is no second authentication path to test.
- Errors are typed: `Sealed`, `CeilingReached`, `RateLimited`, `NotFound`, `Refused`,
  `Unavailable`.

### 5.3 `KeyCache`

- Keyed by `(session, fau, class, subject)`. It holds the key in `Zeroizing`, with its lease id and
  last activity time.
- **Only real user requests count as activity.** Middleware touches entries for the user's own
  HTTP requests; SSE keep-alives and background work do not.
- A sweeper releases idle leases. Releasing is not activity.
- Logout and session end release everything for that session. Collaborative editing releases a
  document key when the last client leaves that document.
- The cache never goes to disk, and its `Debug` output prints no key material.

### 5.4 Sealed at the edge

`KeyError::Sealed` becomes a Bokmål page: "Innholdet er midlertidig utilgjengelig. Vi jobber med
saken." Navigation, login and authorization keep working.

### 5.5 First consumers

The two #3418 messages that are refused today start being accepted:

- **The access-request message**, sealed to the FAU's public key. It is readable only through
  `sealing_private` in an admin's session.
- **The invitation message**, under an invitation key.

They are small and already specified, and they prove the whole chain end to end.

## 6. Deployment and alerts

### 6.1 Image

**Ruling:** a separate minimal image, `fau-key-service`, built from the same Dockerfile and commit
as a second target. It carries that binary only: no shell, no backend code.

ADR-001's "one binary, one image, one digest" is about the application's test, migration and
deploy barrier. The key service is a separate trust boundary. Folding it in as a `fau keys`
subcommand would put all the backend's code inside the boundary that must stay reviewable in an
afternoon.

### 6.2 Kubernetes

The manifests are written in card 1 and deployed with the app under #3424, since the app is not in
the cluster yet either.

- Its own namespace, `fau-keys`.
- A StatefulSet with 1 replica.
- A PVC with an explicit `storageClassName: hcloud-volumes` of 1 Gi, because of the two default
  StorageClasses from #3488.
- Non-root, a read-only root filesystem, and `automountServiceAccountToken: false`.
- **NetworkPolicy:**
  - ingress on the API port from the app pods only;
  - ingress on the metrics port from monitoring only;
  - no egress at all except DNS.
- **No PodDisruptionBudget**, because with one replica it would block node drains. A reschedule
  simply means sealed, then an alert, then Erik unseals. That is the cost ADR-003 accepted.

### 6.3 Compose

A `keys` service in `compose.yaml` runs in dev mode, with its own volume and the dev
certificates. The app's `depends_on` waits for it to be healthy.

### 6.4 The dev-mode guard

`FAU_KEYS_DEV_ROOT_KEY` is honoured only when the binary is compiled with the `dev` cargo
feature. The release image is built without it, and a test asserts that the release binary
refuses the variable.

### 6.5 Metrics and alerts

The rules go to #3442.

| Signal | Severity |
|---|---|
| Sealed | critical |
| Unseal attempted with a wrong key | critical |
| New distinct FAU keys per hour above the threshold | critical (the mass-decryption signal) |
| `destroy_fau` enqueued, or a queue entry cancelled | critical (ADR-003 decision 7) |
| Destroying a chat epoch younger than 12 months attempted | critical |
| Concurrency ceiling reached | warning |
| Rate limit hit | warning |

## 7. Tests

The plan expands these.

- **Round trips:**
  - every key class round-trips;
  - associated-data binding: a value moved to another row, column or FAU fails to decrypt.
- **Sealing and unsealing:**
  - the service starts sealed and is unsealed only by the correct key (the canary);
  - `init` refuses a store that already has a canary;
  - the release binary refuses the dev-mode variable.
- **Surface:**
  - the route list is pinned, and no endpoint returns more than one key;
  - a client without the right certificate is refused.
- **Leases and limits:**
  - the ceiling is enforced through leases;
  - releasing is not activity;
  - an idle lease expires.
- **Destruction:**
  - a destroyed key is gone from the SQLite file, checked by searching the file's bytes for the
    wrapped value;
  - destroying a young epoch is refused and alerts;
  - `destroy_fau` writes a queue row and removes the live KEK.
- **No key material** appears in metrics or log lines.
- **The #3418 messages:**
  - an access-request message is sealed and readable only through `sealing_private`;
  - an invitation message round-trips.

## 8. What this changes elsewhere

| Where | Change |
|---|---|
| ADR-003 decision 5a | "FAU data key" becomes the record key; a table of key classes is added |
| #3501–#3504, #3419 | Depend on the key-service card |
| #3432 | Depends on the key-replica card |
| #3481 | Gains a fourth Proton Pass item with card 2: the backup root key for the replica |
| #3442 | The alert rules in section 6.5 |
| #3424 | Deploys the `fau-keys` namespace and the internal CA Issuer with the app |
