# Key service: design (OpenBao)

Status: **revised and agreed in chat, 27 September 2026.** It replaces the 26 September version,
which designed a hand-written key service. Erik then asked for battle-proven components over code
FAU owns (docs/planning-decisions.md, "OpenBao replaces the hand-written key service"). This
document implements ADR-003 decisions 5, 5a, 6, 7 and 7b (docs/identity-and-encryption.md) with
**OpenBao's transit engine**, and records what that changes. Rulings the agent made without an
explicit answer are marked **Ruling**.

## 1. Why, and what is phased

Every feature designed on #3500 stores encrypted fields: group names, member names, chat, event
details. The documents migration (#3419) does too. On 26 September Erik chose to build key
management **before** #3501. On 27 September he chose OpenBao over writing it ourselves.

| Card | Scope | Gate |
|---|---|---|
| **#3506 Key service** | Sections 2–7: OpenBao deployed and configured, the backend client and session cache, envelope encryption, the `wrapped_keys` table, the #3418 messages, the deletion job | Blocks #3501, #3502, #3503, #3504 and #3419 |
| **#3507 Key replica** | Replicating OpenBao's key state off the cluster without a snapshot history of keys; the 7-day hard delete executed from the replica side; the #3425 restore rehearsal | **Hard gate before any real FAU data**: #3432 depends on it |

Until #3507 exists, losing OpenBao's volume loses every key and therefore all content. That is
acceptable only while there is no real data.

## 2. Decisions

| # | Question | Decision |
|---|---|---|
| K1 | Build order | Key management first, before #3501 (26 September) |
| K2 | The replica | Phased into #3507, which gates the pilot (26 September) |
| K3 | How the key service trusts the backend | A service identity plus limits, with no membership checks in the key service (26 September). With OpenBao: Kubernetes service-account auth, per-path policies and rate-limit quotas |
| K4 | Build or adopt | **Adopt OpenBao** (MPL-2.0, Linux Foundation) and its transit engine (27 September). Cosmian KMS and HashiCorp Vault were rejected as BUSL; Tink is a library, not a service |
| K5 | How the backend uses keys | **Envelope encryption** (27 September): OpenBao generates a data key; only its wrapped form is stored, in our Postgres; the backend unwraps once per session and encrypts locally. Wrapped keys in OpenBao's KV store was rejected: two stores with no shared transaction, and two deletion paths |
| K6 | Custody | **Proton Pass holds the unseal key only.** The initial root token is revoked after setup and regenerated on demand with `bao operator generate-root` (27 September) |

## 3. Keys

### 3.1 One transit key per unit that must be shreddable

There is no key hierarchy inside OpenBao: transit keys are flat. Each unit we must be able to
destroy on its own gets its own transit key, named so that an FAU's keys can be found together.

| Transit key | Kind | Protects | Destroyed when |
|---|---|---|---|
| `fau-<tenant>-record` | wraps one data key | small fields outside documents: group names, member display names and contact emails, event title/location/description, poll title/description | the FAU is deleted |
| `fau-<tenant>-doc-<document>` | wraps one data key | one document's step batches, checkpoints, title, filenames and bytes | the document is purged (ADR-003 7b) |
| `fau-<tenant>-chat-<YYYY-MM>` | wraps one data key | chat bodies from that Europe/Oslo month | 12 months after the month ends |
| `fau-<tenant>-messages` | encrypts directly | access-request and invitation messages | the FAU is deleted |

- **Key type:** transit's default, `aes256-gcm96`.
  - Each transit key encrypts only a handful of values (wrapped data keys or short messages), so
    GCM's nonce limits are irrelevant.
  - It is also the type `vaultrs` supports.
- **Every transit key is created with:**
  - `exportable = false`;
  - `allow_plaintext_backup = false`;
  - `deletion_allowed = false`. Only the deletion job sets `deletion_allowed = true`, just before
    deleting a key.

### 3.2 Envelope encryption for records, documents and chat

1. **Create.** When a unit first needs a key, the backend asks transit
   `datakey/wrapped/<transit key>` for a 256-bit data key. It stores the wrapped form in
   `wrapped_keys` **in the same transaction** as the first row it protects. Transit
   `datakey/plaintext` returns both forms, so the first use needs no second call.
2. **Use.** The backend unwraps with transit `decrypt` **once per session per unit**, and holds the
   plaintext data key in memory under ADR-003 5a:
   - refreshed by real user activity only;
   - zeroised on logout, idle or session end;
   - a document key is released when the last client leaves the document.
3. **Encrypt locally** with XChaCha20-Poly1305 (RustCrypto `chacha20poly1305`, NCC-audited):
   - envelope: `version ‖ nonce ‖ ciphertext`;
   - associated data: `tenant ‖ table ‖ column ‖ row id`, so a value cannot be moved to another
     row, column or FAU;
   - **OpenBao never sees content.**

`wrapped_keys`:

| Column | Notes |
|---|---|
| `tenant_id` | composite key, as every tenant table |
| `unit` | `record`, `document`, `chat` |
| `scope` | document id or `YYYY-MM`; null for `record`. Not `subject`, which ADR-003 decision 3 reserves for `identity_mappings` |
| `wrapped_key` | `bytea`: transit ciphertext of the data key (the `vault:v1:…` string) |
| `created_at` | |

The primary key is `(tenant_id, unit, scope)`, with a unique index for the null-scope case.

**Why a wrapped key in our database is safe:** ADR-003's original rule came from one global master
key that was never destroyed, so a wrapped key in a backup stayed recoverable. Here the wrapping
key exists **at exactly the granularity it must be shredded at**. Destroying it makes every copy of
the wrapped key useless, in the live database and in every backup.

ADR-003 decision 5 is reworded accordingly: "no **usable** key material in the application
database; only data keys wrapped by an OpenBao transit key that exists at the granularity it must
be shredded at." The schema-review test allows `wrapped_keys.wrapped_key` and nothing else.

### 3.3 Messages: direct transit encryption

Access-request and invitation messages are short and rare, so they do not use envelope encryption.
The backend calls transit `encrypt` / `decrypt` on `fau-<tenant>-messages` directly, with
associated data `table ‖ column ‖ row id`.

- **Access-request messages.** They are encrypted at submission, with no member session, which is
  possible because encrypting needs no key in the backend. They are decrypted only on the admin's
  approval screen, and never for email.
- **Invitation messages.** They are decrypted on the invitation page for the verified recipient
  holding a valid, unused token (the 24 September decision). Resend keeps the message.

This replaces the 24 September designs:
- **The sealing key pair (HPKE) goes.** The property it gave is kept in a different form: "the
  submitting request cannot read back what it wrote" becomes "only the approval screen's code path
  calls decrypt", backed by audit. The backend held both halves of the capability anyway.
- **Per-invitation keys go.** Invitation messages are shredded with the FAU, which was the only
  destruction ever planned for them.

ADR-003 decision 6 is amended to match.

## 4. OpenBao: deployment, custody and access

### 4.1 Deployment

- **The official `openbao/openbao` Helm chart**, standalone mode, one replica. It is a single,
  rarely rescheduled workload, the cost ADR-003 accepted.
- **Integrated (Raft) storage** on a PVC with an explicit `storageClassName: hcloud-volumes`,
  because of the two default StorageClasses (#3488).
- **No Raft snapshots are configured.** A snapshot history of key material would keep destroyed
  keys alive. Replication without that history is #3507's job.
- **TLS on the listener**, with the certificate from a cert-manager internal CA Issuer, separate
  from the ACME issuers.
- **Hardening:**
  - OpenBao has dropped mlock (2.7.0 refuses a config that sets `disable_mlock`, and the 0.29.6
    chart has no `IPC_LOCK` and sets `SKIP_SETCAP=true`), so the control is **no swap on the
    nodes**, to be verified on the cx23 nodes (#3424);
  - non-root, as the chart does;
  - core dumps are disabled by OpenBao itself.
- The UI is disabled.
- A NetworkPolicy allows ingress on 8200 from the app's pods, the deletion job and monitoring only.

### 4.2 Seal and custody

- **Shamir seal with one unseal key** (`-key-shares=1 -key-threshold=1`): the 24 September custody
  decision, mapped onto OpenBao.
- **The unseal key.** Erik keeps it in Proton Pass and enters it with `bao operator unseal` after
  every start. Until then OpenBao is sealed:
  - every transit call fails, so content is unreadable;
  - login and authorization in the backend keep working.
- **The root token.** `bao operator init` prints one; it is used for the initial configuration
  (section 4.3) and then **revoked**. When one is needed again, Erik generates it with
  `bao operator generate-root`, using the unseal key, and revokes it afterwards. No standing
  all-powerful token exists.
- **Erik runs `init`, `unseal` and `generate-root` on his own terminal, never through an agent.**
  Their output is the crown jewels, and an agent transcript persists.

### 4.3 Authentication and policies

Configuration is code: a `bao` script in the repo, applied once with the root token and re-run when
it changes.

- **`fau-app`**, Kubernetes auth bound to the app's service account:
  - allows `transit/datakey/{plaintext,wrapped}/fau-*`, `transit/encrypt/fau-*` and
    `transit/decrypt/fau-*`;
  - allows `transit/keys/fau-*`, **create only** (it can create a key but not read, change or
    delete one);
  - denies everything else: list, export, backup, delete, `config`, `soft-delete` and restore.
- **`fau-keys-operator`**, Kubernetes auth bound to the deletion job's service account:
  - allows key metadata reads, `config` (to set `deletion_allowed`), `soft-delete`,
    `soft-delete-restore`, delete, and list on `transit/keys`, plus its queue mount
    `fau-keys-queue/` (section 5);
  - **no encrypt, decrypt or datakey**, so the job that can destroy keys cannot read anything.
- **Rate-limit quotas** on the `fau-app` role's transit paths. These replace the 26 September
  design's lease-enforced ceiling.
- **Ruling:** one starting quota of 50 requests/second on `transit/`, tuned under #3442.

### 4.4 Audit and alerts

- **A file audit device writing to stdout, collected into Loki.** OpenBao HMACs request and
  response values in audit entries, so key material and plaintext never reach the log.
- **Alerts** (rules to #3442; the names carry no member data, per the internal-supplier record):

| Signal | Source | Severity |
|---|---|---|
| OpenBao sealed | `vault_core_unsealed == 0` | critical |
| Many distinct `fau-*` keys decrypted in an hour | Loki query on the audit log | critical (the mass-decryption signal) |
| Any `soft-delete`, `soft-delete-restore` or key delete | audit log | critical (ADR-003 decision 7) |
| A `chat` key deleted before its month plus 12 months | the deletion job's own check, logged | critical |
| Rate-limit quota hit | `vault_quota_rate_limit_violation` | warning |

## 5. Deletion

The deletion job is a Kubernetes CronJob and an on-demand Job, **written as a `bao` CLI script in
the official OpenBao image.** It is not Rust, and it is our code only as a short shell script. It
runs under `fau-keys-operator`.

| Action | Steps |
|---|---|
| **Delete an FAU** (after ADR-003 7a's confirmation) | Soft-delete every `fau-<tenant>-*` key at once, which stops all use and can be undone with restore. After **7 days**, set `deletion_allowed` and hard-delete each one |
| **Purge a document** | The same flow, for `fau-<tenant>-doc-<document>` |
| **Expire chat** | Monthly: soft-delete then hard-delete `fau-<tenant>-chat-<YYYY-MM>` once the month ended 12 months ago. The script **refuses a younger month** and logs it at critical |

- **Where the 7-day step lives.** The pending hard deletes are the soft-deleted keys themselves;
  OpenBao lists them. In #3506, the same CronJob runs the 7-day hard delete. #3507 decides whether
  it must move to the replica side (ADR-003 decision 7: compromising the key service must not grant
  the ability to cancel).
- **After a transit key is destroyed**, its `wrapped_keys` rows and the ciphertext are useless.
  They are deleted by the application's own purge, which is ordinary tidying and not the security
  property.

## 6. What FAU still owns

| Piece | Size | Built on |
|---|---|---|
| `fau-crypto` | ~50 lines: the local envelope and associated data | RustCrypto `chacha20poly1305` |
| `fau-keys` client | a thin wrapper: create-or-get a unit's data key, unwrap, encrypt/decrypt a message | `vaultrs` 0.8 (transit, Kubernetes auth) |
| `KeyCache` | the session-held keys of ADR-003 5a | in-memory, `zeroize` |
| `wrapped_keys` | one migration, one table | Postgres |
| The #3418 messages | persistence fields and readers | section 3.3 |
| OpenBao configuration | policies, auth roles, quotas, audit device as a `bao` script; Helm values | OpenBao, its Helm chart |
| The deletion job | a `bao` shell script and CronJob | the OpenBao image |

Error handling at the edge: if OpenBao is sealed or unreachable, the API answers the error code
`content_unavailable` (503). The Bokmål source string for the client catalogue (#3439) is
"Innholdet er midlertidig utilgjengelig. Vi jobber med saken." Navigation, login and authorization
keep working.

**Local development and tests:**
- Compose runs `bao server -dev` (in-memory, auto-unsealed, dev root token) plus the same
  configuration script.
- Integration tests use `TEST_OPENBAO_ADDR` and `TEST_OPENBAO_TOKEN`, the way `TEST_DATABASE_URL`
  works.
- Dev mode never runs outside compose or tests.

## 7. Tests

- **Envelope:**
  - every unit's data key round-trips (create, wrap, store, unwrap in a new session, decrypt);
  - a value moved to another row, column or FAU fails to decrypt.
- **Shredding:** after the transit key is deleted, a stored wrapped key cannot be unwrapped, and
  neither can one taken from a database dump made before the deletion.
- **Policies,** tested against a real OpenBao:
  - `fau-app` cannot delete, soft-delete, export, list, read config or set `deletion_allowed`;
  - `fau-keys-operator` cannot decrypt or generate data keys.
- **The deletion job:**
  - soft-delete makes decrypt fail and restore brings it back;
  - hard delete happens only after 7 days;
  - a chat month younger than 12 months is refused.
- **Messages:**
  - an access-request message is stored only as transit ciphertext and read back only through the
    admin path;
  - an invitation message round-trips for the recipient and is unknown to anyone else;
  - resend keeps it.
- **Session cache:**
  - a data key is unwrapped once per session;
  - an idle session is swept and swept keys are zeroised;
  - sweeping is not activity;
  - logout releases only that session.
- **Schema review** allows exactly `wrapped_keys.wrapped_key`.
- **Sealed at the edge:** OpenBao's 503 maps to `Sealed`, and `Sealed` to `content_unavailable`.
  These are tested as two mappings, because sealing the shared dev server would disturb every other
  test.
- **No key material or plaintext in logs:** every type holding a key, token or ciphertext has a
  redacting `Debug`, and each has a test.

## 8. What this changes elsewhere

| Where | Change |
|---|---|
| ADR-003 decision 5 | Wording: only wrapped keys in the application database, wrapped at shredding granularity; OpenBao named as the key service |
| ADR-003 decision 5a | The "FAU data key" is the record data key; the ceiling is rate-limit quotas plus the distinct-keys alert, not an enforced lease ceiling |
| ADR-003 decision 6 | Access-request and invitation messages use direct transit encryption on `fau-<tenant>-messages`; the sealing pair and per-invitation keys are withdrawn |
| ADR-003 decision 7 | The 7-day queue is OpenBao soft delete; where the hard delete runs is #3507's |
| `schema_review` test | Allows `wrapped_keys.wrapped_key` |
| #3507 | Rescoped: replicate OpenBao without a snapshot history of keys, the replica-side hard delete, the restore rehearsal |
| #3481 | The Proton Pass item is the OpenBao unseal key |
| #3442 | The alert rules in section 4.4 |
| #3424 | Deploys the OpenBao chart, the internal CA Issuer, the configuration script and the deletion job |
| internal-supplier-ownership | OpenBao is self-hosted open source, not a supplier; recorded for completeness |
