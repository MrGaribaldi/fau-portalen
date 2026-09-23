# Document ingest: where scanning and conversion run

Date: 10 September 2026. Owner: Infrastruktur, drift og lagringsøkonomi. Favro: #3447.
Status: assessment of Erik's design, with a recommendation. Nothing built.

Decided by Erik, 10 September 2026: **malware scanning runs in-house**, not through an external
scanning API. That closes the supplier question - no member document leaves the cluster, so the
scanner needs no entry on #3409 and no European-ownership decision. Erik's proposed shape: when a
user uploads files, spin up a `cx33` worker that does the scanning and the conversion from other
formats into FAU's Markdown.

## The instinct is right, and for a better reason than capacity

A dedicated worker is worth having, but the argument for it is isolation rather than headroom.
Document conversion means running LibreOffice, PDF tooling and image libraries over files supplied
by strangers, and those are historically the most CVE-dense components in any ingest pipeline. A
worker that holds no database credentials, reads only from the quarantine bucket and writes only
results is a real security boundary - the parser can be compromised without reaching the API, the
database or the servable bucket.

Memory is the second argument. ClamAV keeps its signature database resident, on the order of a
gigabyte before it scans anything, and FAU's nodes are `cx23` with 4 GB that will also be running
the API and eventually PostgreSQL. That does not fit comfortably, and it fits worse on the day a
scan and a database backup overlap.

## What is actually available in hel1, checked against the API

Erik asked for a `cpx33`, chosen because that line is almost always in stock. Two corrections from
querying the account's own server types and `hel1-dc2` availability on 10 September 2026:

**There is no `cpx33`.** The line is cpx11/12, cpx21/22, cpx31/32, cpx41/42, cpx51/52, cpx62. The
nearest type to the intent - 4 vCPU, 8 GB - is `cpx32`.

**And the availability instinct is right, more sharply than expected.** Available in `hel1-dc2` right
now, cx and cpx only:

| Type | vCPU | RAM | Disk | hel1 monthly | hel1 hourly | Available now |
| --- | --- | --- | --- | --- | --- | --- |
| cx23 | 2 | 4 GB | 40 GB | EUR 5.49 | EUR 0.0088 | **yes** |
| cx33 | 4 | 8 GB | 80 GB | EUR 8.49 | EUR 0.0136 | **no** |
| cpx22 | 2 | 4 GB | 80 GB | EUR 19.49 | EUR 0.0312 | **yes** |
| cpx31 | 4 | 8 GB | 160 GB | EUR 17.49 | EUR 0.0280 | **no** |
| cpx32 | 4 | 8 GB | 160 GB | EUR 35.49 | EUR 0.0569 | **yes** |

Only the "2" generation of cpx is stocked, and `cx33` - the type an earlier draft of this document
recommended running always-on - **cannot be created in hel1 today**. That is the same flicker Erik
watched in the cx line on 8 and 9 September, and it is a design constraint rather than a footnote:
**a node pool must name an ordered list of acceptable types, not one type**, or it fails to scale on
exactly the day the catalogue and the stock disagree.

## Why an always-on cpx32 was not the answer either

With `cpx32` as the realistic 8 GB choice, an always-on worker costs **EUR 35.49 a month** against a
current infrastructure bill of about EUR 25.46 - it would more than double the running cost. At
EUR 0.0569 an hour, the same worker used an hour a day costs about EUR 1.70 a month, and the monthly
cap is only reached at roughly 623 hours, so genuinely bursty use is an order of magnitude cheaper.
The earlier draft weighed this against EUR 8.49 for a type that is not in stock; against EUR 35.49
the on-demand argument stands on its own.

What has not changed is that the hard parts are not about the node:

**Provisioning latency changes product behaviour.** Creating a server, running cloud-init and joining
k3s takes minutes. Nobody uploading a PDF waits that long, so the pipeline must be asynchronous with
a queue and visible status no matter how the node arrives. Once it is asynchronous, the node's
existence is decoupled from the request.

**Upstream ships no autoscaler.** There is no `cluster-autoscaler` anywhere in infra-tools, so
on-demand scaling means introducing the Kubernetes cluster-autoscaler with the hcloud provider:
node-pool configuration, a token secret in the cluster, and servers created outside Terraform.

**The failure modes cost money quietly.** A node that fails to join leaves a billed server Terraform
does not know about. A scale-down during a scan kills the job unless disruption budgets and
annotations are right. Both are discovered on an invoice or in a lost upload rather than in a plan.

## Decided, 10 September 2026: three tiers, with the cpx32 as escalation

Erik's refinement, and it is simpler than a node per batch:

1. **Single files scan on an existing node**, inline, paying the cost in RAM.
2. **Bulk uploads are processed once a day**, as one batch.
3. **If the existing node is fast enough, it does the bulk batch too.**
4. **If the queue is large enough, a `cpx32` is created for the work and killed afterwards.**

So the ephemeral node stops being the default path and becomes an escalation - which removes the
provisioning latency from the common case entirely, because a single upload never waits for a
server to be created. That is a better design than the one this document proposed.

### The RAM price, stated concretely

The existing nodes are `cx23` with 4 GB, and only `cworker-1` is a plain worker: `master` runs the
control plane and etcd, and `vpn-router` keeps a `NoSchedule` taint. So "an existing node" means
`cworker-1`, which will also host the API and eventually PostgreSQL.

A resident `clamd` holds its signature database in memory - on the order of 1 GB before it scans
anything. Against 4 GB shared with the k3s agent, ingress-nginx, the API and a database, that is the
whole margin. Two ways to pay less for the same guarantee:

- **`clamscan` per file instead of a resident `clamd`.** No permanent reservation; the cost moves to
  a roughly 1 GB spike and several seconds of database loading on every scan. For occasional single
  files that is the better trade. For a daily bulk batch it is clearly worse, since the database
  would reload per file.
- **A second `cx23` at EUR 5.49/month** dedicated to ingest, concurrency one. Cheaper than a
  permanently idle `cpx32` at EUR 35.49 and it keeps the parser off the node that holds the database.
  Worth comparing against the RAM squeeze on `cworker-1` before committing to inline scanning.

The recommendation is to measure before choosing: run `clamscan` inline first, note how long a scan
takes and what it does to `cworker-1`'s memory, and let that decide whether a resident daemon, a
second small node, or the escalation threshold is what changes.

### The escalation threshold needs a number, from measurement

"Large enough" has to become a rule, and it should be expressed in **estimated work, not file
count**: a `cpx32` takes minutes to create and join, so escalating is only worth it when the queue
represents more work than the provisioning cost - a first cut is escalate when the estimated batch
exceeds about fifteen minutes of scanning, revised once real timings exist. The daily bulk window is
where this decision naturally lands: the batch's size is known before any of it starts.

### The four mechanics still apply when it does escalate

1. **"Shut down" must mean delete.** Hetzner bills a server while it exists, powered off included.
   Create and destroy, not start and stop - so the worker holds no state worth keeping. Confirm
   against the first invoice.
2. **Batch within the window.** The hourly rate is EUR 0.0569 and provisioning is minutes; one node
   drains the whole day's queue rather than one node per file.
3. **No public IPv4.** EUR 0.50/month, and it keeps billing if it outlives the server. Unnecessary:
   the private network already routes `0.0.0.0/0` to the vpn-router at `10.0.1.254`, so an
   internal-only worker still reaches the internet for signature updates. The security argument is
   larger than the saving.
4. **Stale Node cleanup is already handled** by the CCM applied the same morning, which removes Node
   objects whose servers no longer exist.

## What the scanner decides, and what happens to a quarantined file

Erik's rule: the scan either **clears** the file for processing or **quarantines** it. Cleared files
are converted to Markdown keeping formatting. For a quarantined file, see whether the problem is a
macro or something else easily disabled, and if so convert the text anyway; otherwise tell the user
the file is infected and cannot be processed.

That intent is right and the second half needs a safe formulation, because **a malware detection is
not a description of what is wrong.** ClamAV returns a signature name - something like
`Doc.Downloader.Emotet-9876543` - not "there is a macro here you could remove". Deciding "this is
only a macro" by reading a signature name would be guesswork on exactly the input where guessing is
expensive.

The formulation that gets Erik's outcome without that guess: **do not ask the scanner what is wrong;
ask whether the threat can survive our conversion.** The pipeline already strips active content and
produces Markdown, which is text. A macro cannot cross that boundary - it is not carried into
Markdown, so a document whose only malicious element is a macro is rendered harmless by conversion
itself rather than by us disabling anything.

What does not survive that reasoning is the case where **the conversion is the attack**: a malformed
PDF or image crafted against the parser that opens it. There, converting is precisely the thing not
to do. So the rule is conservative and file-type based rather than detection-name based:

| Quarantine cause | Action |
| --- | --- |
| Macro or script embedded in an office document (VBA, OLE object, JS in PDF) | Convert **in isolation**, on the ephemeral worker, never on the node holding the database. The original stays quarantined and is never served. |
| Detection on a file type whose risk is the parser itself - malformed PDF, image, font | **Refuse.** Tell the user. Do not open it to convert it. |
| Executable, script or archive containing one | **Refuse.** Nothing in FAU's document model needs it. |
| Detection we cannot categorise | **Refuse**, and log it for review. Default is refuse, not convert. |

Two consequences worth fixing now rather than later:

- **The quarantine-retry path is the best argument yet for the ephemeral node.** Converting a file
  that is known to be infected is exactly the work that should happen on a machine with no database
  credentials, no public address and a short life. That gives the `cpx32` a role beyond capacity.
- **The user-facing message and the audit record both matter.** A refusal should say what to do next
  - upload a clean copy, or ask for help - rather than only reporting failure, because the FAU member
  uploading a ten-year-old archive is not the attacker and may not have a clean copy. And when a file
  is converted despite a detection, that decision belongs in the audit trail: someone later asking
  why a document looks different from the original needs an answer. Retention of quarantined
  originals belongs with #3426.

## What "keeping formatting" can mean

"Convert the text into markdown, keeping formatting" needs its limits written down, because Markdown
cannot express everything a Word document contains and silent loss is the failure mode that matters
for minutes and budget annexes.

Representable, and expected to survive: headings, bold and italic, ordered and unordered lists,
links, footnotes, block quotes, code blocks, and simple tables. Not representable, and therefore
lost or approximated: page layout, columns, text boxes, fonts and sizes, tracked changes, comments,
merged-cell tables, embedded spreadsheets and drawings.

The rule that keeps this honest is the one already recorded: the uploaded original is the
authoritative artifact, stored immutably, and the Markdown is a derived working copy. Anything shown
as an official record links to the original, so a lost merged cell is a display limitation rather
than a lost record. A conversion that hits something it cannot represent should say so on the
document rather than dropping it quietly.

## Conversion target: HTML, and when that is genuinely a security control

Erik's proposal, 11 September 2026: convert DOCX to **HTML** rather than Markdown, because HTML
imports easily into the editor and because the conversion itself may strip exploits.

Both halves hold, the second one conditionally.

### Why HTML is the better import format

The selected editor is Lexical (lexical.dev) in a React page; the rest of the
application stays Rust-rendered HTML/HTMX. Earlier Tiptap/ProseMirror assumptions
are superseded by #3493. HTML is an import intermediate: sanitize it, map supported
structure into the bounded Lexical profile, and report fidelity losses. HTML can
represent richer structure than Markdown, but actual preservation depends on the
converter and the supported editor profile and must be tested.

**Import format and storage format are distinct.** Schema-constrained structured
JSON is canonical editable content, with encryption under #3484. HTML remains a
sanitized derivative. The exact Lexical wire profile, server validation and Rust
rendering contract are specified in docs/structured-document-architecture.md and
validated through #3490/#3422. Preserve snapshots, audit atomicity and leases.

### The security claim depends entirely on the converter's architecture

There are two kinds of converter and they have opposite security properties.

**Parse-and-rebuild** converters read the document model and emit new HTML from a small set of
elements they understand - `mammoth` is the clearest example, driven by an explicit style map.
Nothing from the original file reaches the output except text and structure the converter chose to
carry. Macros, OLE objects, embedded binaries, field codes and remote references are dropped because
the converter has no code to emit them. This is sanitisation by construction, and it is what makes
Erik's point true.

**Render-and-export** converters - LibreOffice `--convert-to html` - use the same full document
engine an attacker targets, then serialise what they rendered. The output is cleaner than the input,
but the *process* is the exposure, and the HTML is messy: inline styles, font tags, spans carrying
layout rather than meaning.

The framing that keeps this honest: **conversion sanitises the output, never the input.** Whichever
converter runs, it still parses hostile bytes, which is exactly why the isolation boundary and the
ephemeral worker stay necessary. "The converter removes the exploit" is true of the document we
store and false of the process that produced it.

### Three hard requirements that come with DOCX

1. **The converter must fetch nothing.** A DOCX can carry a remote template reference in
   `settings.xml`, external relationship targets, `INCLUDEPICTURE` and DDE field codes. A converter
   that resolves any of them turns an upload into an outbound request from inside our network -
   server-side request forgery, and with SMB-style targets a credential leak. Enforce it rather than
   trust it: the conversion Job gets a NetworkPolicy denying egress. That conflicts with ClamAV
   needing egress for signature updates, so those are **separate Jobs with separate policies** -
   signatures update on a schedule, conversion runs with no network at all.
2. **The output HTML is still untrusted.** Converter output goes through an allowlist sanitiser
   before storage or editor import: no `script`, no event handlers, no `javascript:` or `data:`
   URLs, no `iframe`, `object` or `embed`, and SVG either refused or sanitised as its own format,
   since SVG is script-capable. This is the active-content stripping #3447 already specifies, now
   applied to converted HTML rather than to the original file.
3. **Re-encode images, do not pass them through.** Embedded media is where exploit risk reaches the
   *reader*, because the browser's image parser is the target. Decode and re-encode to a normalised
   format, drop anything that fails, and never emit the original bytes.

Also, because DOCX is a zip: cap entry count and uncompressed size against zip bombs, reject
absolute or traversing entry paths, and refuse encrypted documents outright - they cannot be scanned
or converted meaningfully, and asking the user for the password moves the problem rather than
solving it.

### Recommendation

`mammoth` as the default DOCX converter, for the parse-and-rebuild property rather than for
fidelity, with an explicit style map so the mapping from Word styles to our elements is a reviewed
decision rather than a default. LibreOffice headless only as an opt-in fallback when fidelity
genuinely matters, and only inside the isolated worker with egress denied.

PDF is a different problem and should not be folded in by analogy: PDF-to-HTML is lossy in ways that
matter for records, and PDF parsers are the CVE-heavy end of this field. Keep PDFs as attachments
with text extraction for search, rather than converting them into editable content.

## Converter choice remains open on #3419

Converting DOCX, ODT or PDF into Markdown is lossy - tables, footnotes, images and layout are
where it goes wrong. FAU's documents are minutes and records, so a conversion that silently drops
a table from a budget annex is worse than a conversion that refuses.

The rule that keeps this safe, consistent with the immutable-history premise in #3412 and ADR-002:
**the uploaded original is the authoritative artifact**, stored immutably in the private bucket,
and the Markdown is a derived working copy that can be regenerated when the converter improves.
Anything shown as an official record links to the original. The converter choice - pandoc,
LibreOffice headless, or a purpose-built extractor - and the fidelity expectations belong on #3419
with the Markdown and text-import work, not here.

## What this does not change

Scanning does not replace the active-content stripping already specified on #3447. A stripped file
can still be malware and a clean-scanning file can still carry active content, so both stay in the
pipeline, and quarantine-not-pass-through applies to either failing. The scanner's own signature
updates need egress from the worker, which is the one outbound dependency this design adds.
