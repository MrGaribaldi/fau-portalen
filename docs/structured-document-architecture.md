# Document, collaboration and versioning architecture

Status: proposed, 22 September 2026. Deliverable for #3490. **Replaces the 11 September version of
this document in full**, which specified a bounded Lexical JSON profile with exclusive editing
leases. Built from Erik's brief of 22 September, written with Sol, and the review of it recorded in
docs/planning-decisions.md.

Amends the accepted tenant, role and history model on #3412 without reopening it, and depends on
ADR-003 (docs/identity-and-encryption.md) for encryption and key custody.

## 1. How we got here, in one paragraph

The editor has been decided three times. ProseMirror/Tiptap was the original assumption; Lexical
replaced it on 11 September (#3493) because its playground matched the editing experience Erik
wanted; ProseMirror returns now. The deciding reason is collaboration: the product needs several
people working on the same document over time, and ProseMirror ships the collaboration model we
are actually using, with a decade of production behind it. Two supporting reasons: ProseMirror's
node extensibility is what decision blocks, action points and future document types need, and Erik
prefers to avoid Meta-originated dependencies where there is a comparable alternative. The
supporting reasons did not decide it; collaboration did.

The reversal is recorded rather than quietly applied, because #3493 and #3415 are both accepted
cards naming Lexical and the board must not disagree with the code.

## 2. What the product actually needs

Recorded because the first version of this architecture was built on a narrower reading, and the
narrower reading produced the wrong answer.

FAU-portalen is not a minutes archive with an editor attached. Documents are the working surface
for everything an FAU does: an application for funding a new basketball court, drafted by several
parents over weeks; a plan for the annual event, which generates a task list this year and a
different one next year; the minutes, which are the simplest case rather than the typical one.
Spreadsheets and presentations are wanted later, so that an FAU has one place for its work rather
than three.

Two consequences follow immediately and both bind the schema:

- **Collaboration is a requirement, not a refinement.** Several people editing one document over
  time is the normal case.
- **Anything that must be listed across documents cannot live only inside a document.** Document
  bodies are encrypted, so a query that scans them is impossible by construction - the same reason
  there is no cross-FAU search. Tasks, deadlines and anything else the product must aggregate are
  application rows that reference a document, never nodes buried inside one.

## 3. The stack

```text
Tiptap OSS (editor UI)
        |
ProseMirror (document model, schema, transforms)
        |
prosemirror-collab (client: rebasing against a central authority)
        |
   WebSocket
        |
Rust authority (ordering, persistence, broadcast, authorization)
        |
PostgreSQL
```

No Yjs. No CRDT. No Hocuspocus. No Node in production - the collaboration authority is part of the
Rust backend, which keeps the standing decision of 11 September intact and puts authorization in
the same process as every other authorization decision.

React remains scoped to the editor route. The rest of the application stays Rust-rendered HTML with
htmx, per #3415.

## 4. Why a central authority rather than a CRDT

A CRDT was the brief's proposal and was rejected on 22 September. The reasoning, kept because it
will be asked again:

- **Offline editing is out of scope.** The product is online-only for now, and offline and
  peer-to-peer operation is the advantage a CRDT buys. Without that requirement it is cost without
  return.
- **History must be editable.** Erik requires force-undo, removal of content from history, and
  purging whole documents. A CRDT keeps deleted content as tombstones by design, and garbage
  collection does not retroactively purge update blobs already written to disk. An ordered step log
  under a single authority can be rewritten from a checkpoint; a CRDT history cannot.
- **It fits the audit model we already accepted.** #3412's history is change sets committed
  atomically with their audit entries, each with one author and one timestamp. A step batch has
  exactly that shape. A stream of CRDT updates has neither an author nor a natural transaction
  boundary.
- **It fits encryption.** A step batch is one blob with one key, one author and one timestamp.

The trade accepted in exchange: the authority must be reachable to edit. That is acceptable because
the product is online-only.

## 5. Division of labour, and what the server does not do

The client applies steps and materialises the document. The server never applies a ProseMirror
step and contains no ProseMirror transform implementation - that was considered and rejected as
the largest risk in the design, because a Rust reimplementation that diverges from the JS semantics
produces silent disagreement between server and clients.

**The Rust authority:**

1. authenticates and authorizes the connection (section 11);
2. assigns version numbers and accepts or rejects step batches on a version match;
3. persists accepted batches, encrypted;
4. broadcasts accepted batches to other connected clients;
5. stores client-submitted checkpoints;
6. sanitises and stores published output (section 9).

**The clients:** apply and rebase steps, materialise the document, render for display and for PDF
export, and submit checkpoints.

**The accepted consequence.** Because the server does not replay steps, a client-submitted
checkpoint could in principle disagree with the step log. The checkpoint is authoritative for
publication and recovery; the step log is history. Reconstructing history from scratch therefore
needs a browser, which is awkward during a disaster restore and is written down here rather than
discovered then. Under the threat-model boundary in section 12 this is acceptable: the risk is a
bug, not a forgery.

## 6. Persistence

Illustrative rather than final; column names are for the implementation to settle.

```text
documents
  id, fau_id, type, model_version, title_encrypted, status,
  created_at, updated_at, current_version, document_key_id

document_steps
  id, document_id, version, steps_encrypted, author_id, created_at, client_id

document_checkpoints
  id, document_id, version, doc_encrypted, created_at, created_by

document_revisions
  id, document_id, checkpoint_id, label, kind, created_at, created_by
```

`type` and `model_version` exist from the first migration so that spreadsheets and presentations
reuse this substrate rather than forcing a second one. The authority never interprets a step, so it
is model-agnostic already; only the editor and the renderer differ per type.

**Batching.** Steps are batched per client submission rather than per keystroke. One row per
accepted batch, with its author.

**Checkpoints.** Written on a cadence the implementation picks - a version interval, an idle
interval, and on publication. A checkpoint bounds replay length and is what a client loads on open.

**Compaction is also the erasure mechanism**, which is the part most easily missed: truncating the
step log behind a checkpoint is what bounds how long deleted text survives. It is not only a
performance concern, and the retention policy for the step log is therefore a privacy decision.
Proposed: keep steps for the life of the revision history the FAU can browse, compact behind the
oldest revision a user can reach.

**Revisions** are the user-visible unit: a stable historical state, created automatically at
checkpoints, explicitly by a user with a label, and always at publication.

## 7. Encryption, and per-document keys

ADR-003 governs. One extension is added here and it is new: **each document has its own data key,
wrapped under the FAU key-encryption key.**

```text
document content ── encrypted under ──> document key
document key     ── wrapped under  ──> FAU key-encryption key (key service only)
FAU KEK          ── destroyed to   ──> crypto-shred the whole FAU
document key     ── destroyed to   ──> crypto-shred one document
```

This is what makes Erik's requirement to purge a whole document *provable* rather than best-effort.
Destroying one document key makes that document's steps and checkpoints unrecoverable everywhere,
including in backups already written, with no cleaner to run and nothing to find. It composes with
the queued-deletion machinery in ADR-003 decision 7 at no additional design cost.

**Encrypted:** step batches, checkpoints, document titles, comment bodies and quoted anchor text,
uploaded bytes and original filenames.

**Plaintext:** document identifiers, type, version numbers, timestamps, author identifiers, task
rows' scheduling metadata, comment thread structure and resolution state, and audit event types.
This is what the product must query, and it is the same boundary decision 6 of ADR-003 already
drew.

**The collaboration authority is inside the key boundary.** It holds a document key while a
document is open, so ADR-003 decision 5a's rules apply to it: memory only, refreshed by real
activity, zeroised when the last client disconnects, and counted against the concurrency ceiling.
The ceiling now bounds open documents rather than open sessions, and the previous trigger for
zeroising - lease release - no longer exists and is replaced by last-client-disconnect.

## 8. Anchors, comments and tasks

Comments and tasks anchor into the text by the same primitive: **a position plus the version it was
recorded at**, mapped forward through subsequent steps using ProseMirror's `Mapping`. Plain offsets
are insufficient, as the brief correctly said; the CRDT-free answer is mapping, which is
deterministic and replayable, so an anchor can be reconstructed at any revision rather than
depending on editor-internal state.

Specify in implementation: serialisation of anchors, behaviour when all anchored text is deleted
(orphaned, not silently relocated), overlapping anchors, and anchor behaviour when viewing an
older revision.

**Comments** are application data: threads, replies, author, timestamps, resolution and reopening,
permissions and audit. Bodies and quoted anchor text are encrypted. Comments are never included in
published output.

**Tasks are rows that point into a document, not nodes inside it.** Erik's annual-event case is
what settles the direction: a plan generates a task list for 2027 and a different one for 2028, and
the document must not be bound to either. So a task list references a document and a period, and a
task carries an anchor to the passage it came from. The document renders them inline; the database
can list them across documents, which an encrypted body could never support.

## 9. Publication

The publishing user's client renders the document and exports HTML and PDF. **The server sanitises
what it receives** with an allowlist HTML sanitiser before storing it - it does not trust the
client's sanitisation, because under section 12's threat model the publisher is exactly the person
whose paste may have carried something they never noticed. The stored artifact is then served as a
static file with no JavaScript.

- Served from a **separate origin** to the application, so residual script cannot reach the app's
  session context. The same reasoning #3435 already applies to stored files.
- Strict Content-Security-Policy on the published page.
- No JavaScript required to read a published document, which also serves accessibility, print and
  search.

**Two consequences recorded now rather than discovered later.**

PDF export runs in the publisher's browser, so the same document published by two different people
produces slightly different PDFs - fonts, pagination, browser version. Acceptable for an
application text or a set of minutes; it means a published PDF is not a deterministic artifact and
should not be treated as a byte-stable archival record.

**Published output is outside the encryption boundary by design** - it is a released derivative -
which means **crypto-shredding does not erase it.** Destroying a document key makes the private
source unrecoverable and leaves the published HTML and PDF untouched. Publication therefore needs
its own deletion path, and the bucket holding published artifacts must permit real deletes rather
than carrying the permanent-deletion-denied policy used on `fau-tfstate`. The same constraint, for
the same reason, as the key replica in ADR-003 decision 7.

## 10. Import

MVP accepts **HTML and pasted content**. DOCX upload is deferred, and the workflow is that a user
exports from Word or Docs to HTML, or pastes the document in. This exists to avoid making DOCX
parsing a launch requirement, and it supersedes nothing about safety.

**Paste is the primary path and carries the same risk as upload.** A parent pasting a paragraph
from a municipal website brings whatever that page contained. The normalisation layer therefore
runs on paste as well as on file import.

Normalisation: security sanitisation, then Word/Docs cleanup, then semantic normalisation into the
supported schema, then the ProseMirror parser. Deterministic output. Reusable later for DOCX
conversion, so DOCX when it arrives converts to HTML and re-enters here.

**This layer sits inside #3447's pipeline, not beside it.** Everything already decided there
applies unchanged: magic-byte type detection, the accept list, malware scanning on the ephemeral
worker, quarantine on failure, resource limits, the original retained and never served, and the
source allowlist for automatic fetching. One point of contradiction with the brief resolved
explicitly: **images are re-encoded, never passed through**, because the reader's image parser is
where exploit risk lands.

One practical gap to handle in implementation: Word's HTML export writes images into a sidecar
folder rather than inline, so a user uploading only the `.htm` loses every image. Accept a paste or
a zip, or tell the user plainly.

## 11. Authorization, and the requirements inherited from #3420

Every collaboration connection verifies, server-side and independently of the client: authenticated
identity, FAU membership, access to that document, and write permission. A read-only member's step
batch is rejected by the authority.

This is a correctness requirement before it is a security one - without it, a client bug silently
corrupts a document, which will happen long before anyone attacks us.

**Authorization is re-evaluated on every accepted batch, not at connection establishment.** This is
the requirement most easily lost when a lease model becomes a WebSocket model: a member whose role
is revoked mid-session must have their next batch refused, on a connection that was legitimately
authorized when it opened. It is the same rule ADR-003 decision 4 already states for long sessions -
the tenant model re-evaluates authorization per operation rather than at login - and a revoked role
must die inside a live socket exactly as it dies inside a live session.

Also: WebSocket origin checks, payload caps, a maximum document size, and rate limits.

### Inherited from #3420

#3420 specified exclusive editing leases and was closed on 22 September 2026 when collaborative
editing replaced that model. Four of its five acceptance criteria survive in changed form and are
carried here so they are not lost with the card.

| #3420 required | Where it lives now |
| --- | --- |
| Closing the editor releases the lock | No lock exists. Last client to disconnect triggers a checkpoint and zeroises the document key handle (ADR-003 decision 5a) |
| 15 minutes without changes expires the lease | Idle timeout on the key handle and an idle-triggered checkpoint. The interval is an implementation choice within the tens-of-minutes bound |
| An administrator can force-release | No lease to release. The equivalent need is met by role revocation taking effect on the next batch, per the rule above |
| The last server-saved state is kept and remnants discarded | Accepted batches are durable; rejected batches are rebased or discarded on the client. "Saved" means server-acknowledged and nothing in the interface may claim otherwise |
| A revoked token cannot write afterwards | Per-batch authorization, above. This is the one that would have been silently dropped |

What does not survive, deliberately: lease acquisition, renewal, expiry, takeover, and the
"someone else is editing this" interface.

## 12. Threat model boundary

Set by Erik, 22 September 2026, and it scopes several decisions above.

> We defend against mistakes and against casual misuse by people who have legitimate access. We do
> not claim to withstand a determined attacker who is already inside, and we never describe the
> product as secure against one.

This is the same discipline as ADR-003's rule against writing "we cannot see your data": do not
claim a property we cannot back.

What it justifies: server-side sanitisation at publication and on import, because the realistic
case is an unaware parent pasting or forwarding something they never inspected. What it scopes
down: adversarial hardening against authenticated members - basic payload caps and rate limits,
not fuzzing of malformed step batches. What it leaves unchanged: the realistic malicious member is
a compromised honest one, which ADR-003's second-factor gate and invitation notifications already
address.

## 13. Deferred

Live presence and cursors. Offline editing. Track changes and suggestion mode - kept possible, not
built, and noting that open-source ProseMirror track-changes implementations are thin. DOCX upload
and bulk import. Spreadsheets and presentations, which will need their own editor and model but
reuse this substrate. The agent edit API, which needs server-side materialisation and should wait
until there is a reason for it. Word round-trip, pagination and complex print layout.

## 14. What implementation must still decide

Exact Tiptap packages and extensions. Checkpoint cadence and step-log compaction thresholds. Step
batch size limits and maximum document size. WebSocket topology across multiple application
instances, and whether broadcast needs Postgres LISTEN/NOTIFY or a lighter in-process path at
current scale. Anchor serialisation. Revision retention. Reconnect and conflict-resolution
behaviour surfaced in the UI, including the distinction between locally applied and durably
persisted - "saved" must mean server-acknowledged. Observability for rejected batches and rebase
frequency.

## 15. Test matrix

| Scenario | Expected result |
| --- | --- |
| Two clients submit batches at the same version | One accepted, one rejected and rebased; no lost edit |
| Read-only member submits a step batch | Rejected by the authority |
| Member's role revoked while their socket is open | Next batch refused; connection authorized at open is not sufficient |
| Last client disconnects from a document | Checkpoint written; document key handle zeroised |
| Document idle past the timeout with a client still connected | Key handle expires; checkpoint written |
| Client believes it saved while the batch was rejected | Interface must not show "saved" until the authority acknowledges |
| Member of another FAU connects to a document | Refused |
| Client submits a checkpoint that fails schema validation | Rejected and quarantined; step log unaffected |
| Published HTML containing script, event handlers or remote references | Sanitised before storage; never served |
| Published page loaded with JavaScript disabled | Fully readable |
| Document key destroyed | Steps and checkpoints unrecoverable, including in existing backups |
| Document key destroyed after publication | Published artifact still present; removed by its own deletion path |
| Comment anchor whose text is entirely deleted | Orphaned and shown as such, never silently relocated |
| Task list regenerated for a second year | Both lists coexist against the same document |
| Step log compacted behind a checkpoint | Deleted text no longer recoverable from history |
| Connection lost mid-edit | Client reports unsaved state; nothing claims "saved" until acknowledged |
| Paste from a municipal web page carrying a script | Stripped by normalisation before it reaches the document |

## 16. Sources

Erik's architecture brief of 22 September 2026, written with Sol, and the review recorded in
docs/planning-decisions.md under "Collaboration model decided: ProseMirror central authority, not
CRDT". ProseMirror's collaborative editing guide for the authority model. ADR-003 for encryption,
key custody and deletion. #3412 for the tenant, role and history model. #3447 for ingest.
