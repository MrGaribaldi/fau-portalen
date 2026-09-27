# Archiving an FAU for non-payment: design

Status: **agreed in chat with Erik, 27 September 2026; written spec awaiting his review.** It adds
a tenant lifecycle that sits beside ADR-003 7a's deletion flow (docs/identity-and-encryption.md)
and extends the deletion script of docs/key-service-design.md §5. Rulings the agent made without
an explicit answer are marked **Ruling**.

## 1. Why

An FAU may fall behind on payment. Deleting it at once is the wrong answer: a council that forgot
an invoice, or changed treasurer, should be able to pay and continue without losing its history,
and one that has stopped should be able to leave with its records. Today the only way out is the
deletion flow, whose 7-day window is sized for a governed decision to leave, not for a dunning
period.

This is **not a deletion path and not a replacement for 7a.** It is a paused state that ends in
reactivation, a 7a deletion, or destruction when its time runs out.

## 2. Decisions

| # | Question | Decision |
|---|---|---|
| A1 | How the FAU is locked | Two phases. **Read-only** first, with keys live and an application-level lock. **Soft-deleted** after that, with the FAU's transit keys soft-deleted (Erik, 27 September) |
| A2 | Access while read-only | Everyone can read and export. A notice on every login says the FAU is archived and can be reactivated by paying. Nobody is forced to pay, but everyone who reads sees the nag. The length of this phase is **not promised**: it may be a few weeks (Erik, 27 September) |
| A3 | What survives the soft-deleted phase | **Published pages stay online**, and **a completed export stays downloadable** for its lifetime (A5). An FAU without an export can contact us: we may run one manually, or ask them to pay for a month and export themselves (Erik, 27 September) |
| A4 | Who moves an FAU between phases | **An operator, by hand**, in the MVP. Automation from billing state follows once payment processing exists (Erik, 27 September) |
| A5 | Export lifetime | **90 days** from creation, then its key is shredded (Erik, 27 September) |
| A6 | Memberships during the archive | **Suspended, still active** for the email-retention rule. Emails are kept and reactivation brings everyone back. Memberships end when the FAU is destroyed, and the 3-month retention clock starts then (Erik, 27 September) |
| A7 | Notices | **Transactional email** to the administrators and the recovery contact when the FAU is archived, when it is soft-deleted, and 30 days before destruction. The in-app notice alone reaches nobody once content is locked (Erik, 27 September) |
| A8 | Maximum soft-deleted time | **Ruling:** 365 days counted from the soft delete, which is the queue entry's `soft_deleted_at`. The read-only phase comes on top. The alternative is 365 days from the start of the archive. It is simpler to promise but needs a second timestamp in the queue |

## 3. States

```
active ──archive──▶ archived-readonly ──lock──▶ archived-locked ──365 days──▶ destroyed
  ▲                     │                          │
  └──── pay (instant) ──┘                          │
  ▲                                                │
  └──── pay (operator restores keys) ──────────────┘
Both archived states ──delete request──▶ ADR-003 7a (confirmation) ──▶ 7-day window ──▶ destroyed
```

| State | Reads | Writes, invites | Billing | Keys | Published pages | Export download |
|---|---|---|---|---|---|---|
| archived-readonly | everyone, with the notice | stopped | stopped | live | served | yes |
| archived-locked | none; admins see the notice with the pay and delete choices | stopped | stopped | soft-deleted, except export keys | served | yes, within its 90 days |

- **Reactivation from read-only is instant.** It is an application state change and nothing else.
- **Reactivation from locked needs an operator.** Only `fau-keys-operator` can restore keys: the
  app's policy denies restore by design (key-service-design §4.3). A payment in this phase therefore
  opens a support task, not a self-service flow. That is acceptable while A4 keeps every
  transition manual.
- **The tenant lifecycle column must have room for these states.** The migration that introduces
  tenant lifecycle (the 7a freeze) should use a state enum and not a `frozen` boolean.

## 4. Order of operations for `lock`

The lock is the one step that removes access, so its order matters:

1. **The application moves the tenant to archived-locked.** Authorization now denies every content
   read. The one authorization function (docs/groups-directory-chat-calendar-design.md) is where
   this is enforced, including the SSE stream.
2. **Every backend instance evicts the tenant's keys from its session `KeyCache`.** Without this
   step, open sessions keep plaintext data keys in memory until they idle out, and soft-deleting
   the transit key does nothing to them.
3. **The operator runs `shred.sh archive <tenant>`.** Once the application no longer reads, the
   soft delete is the cryptographic backstop, not the access control.

## 5. Changes to the deletion script

These build on the queue entry's `reason` field, added in #3506 Task 8 (ruling R8).

| Command | Effect |
|---|---|
| `shred.sh archive <tenant>` | Soft-delete every `fau-<tenant>-*` key **except** `fau-<tenant>-export-*`, and queue each with `reason=archive` |
| `shred.sh restore-fau <tenant>` | Restore every soft-deleted key of the tenant **whose queue reason is `archive`**, and unqueue it. It must never restore a key queued as `document` or `chat-expire`: payment must not resurrect a purged document or an expired chat month. One `shred: CRITICAL` line per key |
| `shred.sh export-expire` | Soft-delete and queue (`reason=export-expire`) every `fau-*-export-<id>` older than 90 days. The age comes from the UUIDv7 id's timestamp (ADR-002), so no extra state is needed |
| `finalize` | The window depends on the reason: **365 days** for `archive`, 7 days for everything else |

Rules that fall out of the existing commands:

- **A deletion request during the archive re-queues.** After 7a's confirmation, `shred.sh fau`
  rewrites each `archive` queue entry to `reason=fau` with `soft_deleted_at` set to now. The
  365-day window becomes the 7-day one, counted from the confirmation. `soft()` skips a key that
  is already soft-deleted and queued. The #3506 final review added a branch that re-queues a
  soft-deleted key with no queue entry. This rule is a third branch: an existing `archive` entry
  is rewritten.
- **Chat expiry keeps its promise during the archive.** A chat month under `reason=archive` that
  reaches 12 months must be destroyed on time. So `chat-expire` rewrites the entry to
  `reason=chat-expire` (keeping `soft_deleted_at`) rather than skipping the key. Otherwise a locked
  FAU would hold chat past its 12 months for up to a year.
- **Full deletion covers exports.** `fau <tenant>`'s `fau-<tenant>-*` match includes
  `fau-<tenant>-export-*`, which is intended.
- **`archive` is a bulk soft delete, so it fires the critical alert** (key-service-design §4.4).
  That is expected, and the runbook says so. A bulk soft delete nobody expected is exactly what the
  alert is for.

## 6. The export and its key

- Each export is one bundle in private S3, encrypted under a data key wrapped by its own transit
  key, `fau-<tenant>-export-<export-id>`, where the id is a UUIDv7. The bundle is never stored
  decrypted: a plaintext copy of an entire FAU at rest would be the largest leak surface in the
  system.
- Its own key is what lets the bundle outlive the lock (A3), and it is shredded on its own 90-day
  clock (A5).
- **Only administrators download it.** The link is not public, unlike published pages.
- The export format itself is out of scope here. prosjektgrunnlag.md requires one that "an FAU can
  archive or move to another solution".

## 7. Published pages

Published output is **outside the encryption boundary by design**
(docs/structured-document-architecture.md §9), so it needs no key change to stay online through
the lock. It is removed by its own deletion path **at destruction**: when the archive's 365 days
end, or when a 7a deletion finalizes. The destroy step must call that path, or published minutes
would outlive the FAU indefinitely.

## 8. Notices (A7)

| When | To | Says |
|---|---|---|
| archive | administrators, recovery contact | archived for non-payment; read-only; pay to reactivate; export now |
| lock | the same | content locked; published pages stay up; any export stays for its 90 days; the destruction date; contact us to reactivate |
| 30 days before destruction | the same | the destruction date; the last chance to pay |

**The backend cannot read the queue.** `fau-app` is denied `fau-keys-queue/` by design, and that
denial is tested. So the application records the lock time on the tenant row when it moves the
FAU to archived-locked. It computes the destruction date as that time plus 365 days. The operator
runs `shred.sh archive` straight after the state change, so the two clocks agree to within the
operator's delay, and the script's clock, which is the later of the two, is the one that destroys.
The notice promises the application's date, so the FAU is never destroyed before it.
These are transactional emails through the email provider (#3410), in Bokmål source strings with
the localisation mechanism (#3439). They are not the in-app notifications that the #3500 design
keeps in-app only.

## 9. Open

- **The legal basis for keeping an unpaid customer's data.** We are the processor. Retaining data
  for up to a year after the customer stops paying must be written into the terms and the DPA
  (data processing agreement): how long, what they can still reach, and when it is destroyed.
  This design assumes those terms say so, and it must not ship before they do.
- **The replica (#3507).** A key soft-deleted for 365 days on the live service must stay
  restorable on the replica for as long. The replica's window must follow the queue reason, not a
  fixed 7 days.
- **Price for a manual export** (A3), if any.

## 10. Testing

- The script gets the same kind of tests as `test-shred.sh`:
  - `archive` skips export keys;
  - `restore-fau` restores `archive` entries only, and leaves a purged document soft-deleted;
  - `finalize` holds an `archive` entry for 364 days and destroys it at 365;
  - `fau` during the archive re-queues to 7 days;
  - `chat-expire` during the archive destroys a 12-month-old month on time;
  - `export-expire` honours 90 days from the UUIDv7 time.
- The application gets these tests:
  - a locked tenant is denied on every read path and the SSE stream;
  - lock evicts cached keys;
  - reactivation from read-only restores access without touching OpenBao;
  - export download is admin-only and works while locked.
