# ADR-002: URL scheme and public addressing

Status: proposed, 9 September 2026. Amends ADR-001 (docs/repo-container-contract.md), which
reserves `/`, `/app/`, `/api/v1/`, `/assets/` and `/health/` but does not define public
addressing. Decisions recorded here were taken with Erik in conversation on 9 September 2026 and
are logged in docs/planning-decisions.md.

## Why this document exists

The path-addressing decision on #3443 — FAU-er addressed by path, not subdomain — initially put
municipality names at the root, sharing a namespace with ADR-001's reserved paths and with every
product page we would ever add. That is the problem this document set out to solve, and the `/fau`
prefix below removes it by construction: we never mint a root segment at all. #3441's outreach
plan then puts these URLs into emails that people click months later, so two properties matter
more than prettiness:

1. A stale link must never resolve to the **wrong** FAU. Failing visibly beats guessing.
2. Nothing we send by email may depend on a name that can change.

Norwegian administrative reality forces this. Municipality names are not unique — Herøy is the
name of two municipalities, in Møre og Romsdal and in Nordland. Kommunenummer are county-prefixed
and are reassigned wholesale by county reform. Both are also **reused**: the 2017 merger of
Sandefjord (0706), Andebu (0719) and most of Stokke (0720) produced a new municipality numbered
0710 Sandefjord, and 0716 belonged to Våle before it was given to Re in 2002. So neither the name
nor the number is an identifier, and a naive slug can silently point at a different entity.

## Decisions

### 1. Identity is ours; official codes are attributes

Every entity that can appear in a URL — tenant, school, municipality, document, invitation — has
a UUID assigned by us. Kommunenummer, organisasjonsnummer and names are versioned attributes in
the register, never keys. This is the only assumption the cases above do not break.

Document IDs are UUIDs specifically so that a shared link is unambiguous: per-tenant sequential
IDs would make `/app/dokumenter/1001` valid in two different FAU-er for a member of both.
Recommended UUIDv7 rather than v4 — same uniqueness, time-ordered so it does not fragment the
Postgres index on a growing table. The tradeoff is that v7 embeds a creation timestamp, which
leaks nothing meaningful to someone who can already open the resource. #3412's composite
`(tenant_id, id)` keys are unchanged; this defines what `id` is, not the integrity model.

### 2. Two link forms, with different jobs

```
PRETTY, mutable — for humans and search engines
  /fau/3911-faerder/hosle-skole
  always reflects current names; 301s when anything is renamed

PERMANENT, opaque — for anything we send, and for machines
  /s/<school-uuid>
  never changes, never reused, never needs a redirect
```

Everything we put in an email, a QR code or another system uses the permanent form. Slugs exist
for the address bar and for search. Signup links sent in outreach are therefore
`/bli-med/<school-uuid>`, never a slug.

### 3. The municipality segment is `<kommunenr>-<navn>`

Number and name each change independently, but never both at once, so the **pair** is a compound
natural key that is unique in practice and, more importantly, fails safe: a stale pair cannot
match a current pair, so staleness is always detectable rather than silently wrong.

```
/fau/3911-faerder        Færder, Vestfold
/fau/1515-heroey         Herøy, Møre og Romsdal
/fau/1818-heroey         Herøy, Nordland          same name, different place
/fau/0716-vale           Våle, until 2002
/fau/0716-re             Re, from 2002            same number, different era
/fau/5501-tromsoe        /fau/1508-aalesund
```

Transliteration is the correct reversible form — `æ→ae`, `ø→oe`, `å→aa` — giving `faerder`,
`heroey`, `tromsoe`, `aalesund`. This deliberately differs from the lossy forms the
municipalities use in their own `kommune.no` domains; the number prefix means we are not trying
to mirror those addresses.

The canonical path is lowercase. Paths are case-sensitive, so a capitalised variant would be a
second URL for the same page, splitting search signals and breaking hand-typed links; mixed-case
requests 301 to the lowercase form. The register keeps the display name ("Færder") for rendering.

Residual risk, accepted and recorded rather than solved: nothing in law prevents a future entity
from receiving both an old number and an old name. The history table means we would detect it —
the current holder wins the pair and the prior holder keeps its UUID path — so the failure mode
stays visible.

### 4. Schools are addressed by name alone

`/fau/3911-faerder/hosle-skole`. The municipality pair already scopes them, so organisasjonsnummer
in the path would cost readability on the segment humans actually recognise for no gain that the
permanent `/s/<uuid>` form does not already provide. School slugs are unique within a
municipality, enforced in the register.

### 5. Full URL map

```
PUBLIC (no login)
  /                                landing
  /priser  /om-oss  /hjelp  /personvern      product pages; the root stays ours
  /fau                             browse and search schools
  /fau/3911-faerder                all FAU-er in the municipality + "Mangler skolen din?"
  /fau/3911-faerder/hosle-skole    one school's FAU page + "Opprett FAU" button
  /s/<school-uuid>                 permanent address for the same page
  /bli-med/<school-uuid>           outreach signup link, school prefilled

WORKSPACE (login required)
  /app/                            FAU switcher
  /app/dokumenter/<uuid>           a document; tenant resolved from the resource
  /app/audit                       history for the active FAU

MACHINE
  /api/v1/...   /assets/...   /health/live   /health/ready
```

### 6. Workspace links resolve the tenant from the resource

`/app/dokumenter/<uuid>` carries no tenant segment. The server looks the resource up, verifies
membership, and switches the session's active FAU to match when the member has access. The
alternatives were rejected: an opaque tenant segment adds a meaningless component to every
workspace URL, and a tenant slug would put renames back in the path of live bookmarks, which is
what keeping slugs public-only avoids.

Consequences that must be honoured: the app has to show clearly when opening a link changed which
FAU you are viewing, and a resource in an FAU you do not belong to returns **404, never 403**, so
the URL cannot confirm that the resource exists. Authorization is unchanged — membership is
verified per request regardless of how the tenant was selected (#3412).

### 7. Locale prefix

Bokmål is served unprefixed, so today's URLs are final and nothing changes when Nynorsk arrives
(#3439, where Nynorsk is design-only for now). A locale prefix, when introduced, is the outermost
segment: `/nn/fau/3911-faerder/hosle-skole`, with `/nb/...` accepted and 301'd to the unprefixed
form so each page has one canonical URL. Because the prefix sits outside `/fau`, it cannot collide
with a municipality slug. Workspace paths take no prefix; a signed-in user's locale comes from
their account.

### 8. Schools we do not know about

New public schools are established and private schools approved continuously, so the register
will always lag. The municipality page carries "Mangler skolen din?", which asks for the school
name and a link to the **establishing decision on an official domain**:

- a public school: the municipal decision or minutes;
- a friskole: the Utdanningsdirektoratet approval under friskolelova.

The record stores the URL, the document title, the retrieval date and which kind of decision it
is, and the school page displays it as provenance ("Opprettet etter vedtak i Færder kommune,
12.03.2026"), matching the source-per-row requirement on #3441.

**A copy of every linked document is kept.** Municipal and Udir sites reorganise constantly, and
evidence behind a dead link is no evidence. The fetched document is stored in private object
storage under the #3435 rules — server-chosen key, ownership recorded in PostgreSQL — together
with its content hash, byte size, content type, fetch timestamp and the HTTP status at fetch
time. Public minutes and Udir decisions are public documents, so archiving them for provenance is
appropriate; the copy is retained as internal evidence and shown to reviewers, while the public
school page links the original. Retention of both link and copy is a #3426 question.

**Automatic fetching is restricted to an allowlist of official sources.** Erik's rule: sources
are subdomains of `kommune.no` and `udir.no`. Matching is on the host, by exact match or
registrable-suffix match — `udir.no`, `*.udir.no`, `*.kommune.no` — after punycode
normalisation, rejecting any URL carrying userinfo. Substring matching would be a hole:
`kommune.no.attacker.example` and `evil-kommune.no.attacker.example` must both fail.

**The allowlist governs fetching, not admission.** Many municipalities publish minutes through
vendor-hosted innsyn and møtekalender portals rather than their own domain, and some
municipalities and county bodies sit on custom domains, so a strict two-domain rule would reject
a large share of legitimate submissions. Therefore:

- an allowlisted URL is fetched and archived automatically;
- any other URL is accepted with the submission but **not fetched**. The reviewer, who is already
  in the loop because every manual submission emails us, opens it and either adds the domain to
  the maintained allowlist or attaches the document.

The allowlist is extended by us over time — known innsyn portals and fylkeskommune domains — and
extending it is a review action, not something a submitter can do.

**Even inside the allowlist the fetcher is constrained**, because an allowlist is not a promise
about the address a host resolves to and does not survive a hijacked or misconfigured record. The
fetcher must: accept `https` only; resolve the host and refuse loopback, link-local and private
ranges in both IPv4 and IPv6; re-check the host against the allowlist and the address rules after
every redirect, and refuse a redirect that leaves the allowlist; cap the redirect count; enforce
a byte cap and a timeout; accept only document content types; and run with restricted egress
rather than from a pod that can reach cluster services. Never render a fetched document inline in
our own origin.

The allowlist is an SSRF control, not a content-trust control: a legitimate municipal site can
still serve a hostile PDF, so ingest sanitisation below applies to allowlisted sources exactly as
it does to anything else.

**Submission notifies us by email** so a human verifies the school before its pretty slug is
issued. The notification goes to the operational address (`fau@ewb-solutions.as` until the domain
on #3434 is settled) through the ordinary outbox and provider from #3410, carrying the submitted
school name, the municipality pair, the decision link, the submitter's account and the timestamp.
Submitted text is untrusted: it is escaped in the notification and never interpolated into HTML
or a header. The form is rate limited per account and per municipality, since it creates both a
tenant and an outbound email.

**Slug consequence.** An unverified school is created immediately and works fully — a real FAU
must not wait on our review — but receives **only** its `/s/<uuid>` address. The pretty slug is
issued on verification. Nobody can squat a real school's readable URL by inventing it first, and
legitimate users are never blocked. The referat link is the evidence in the resulting review
queue.

## Document ingest and sanitisation

Every document that enters the system — a fetched decision document, and equally any file an FAU
uploads under #3419 and #3435 — is untrusted input. PDFs are programmable: JavaScript actions,
`/OpenAction` and additional-action triggers, `/Launch`, embedded files, RichMedia and XFA forms
all execute or fetch on open in some readers. So active content is stripped before anything is
stored or served.

**Two copies, because stripping changes the bytes.** Sanitising alters the file, so a sanitised
document no longer hashes to the source — which would quietly destroy the provenance value that
made us keep a copy at all. Therefore:

- the **original** is stored quarantined, with its hash recorded, and is never served to anyone;
  it exists so we can prove what the municipality or Udir actually published;
- the **sanitised derivative** is what reviewers and users ever receive.

Both are recorded, and the audit entry names what was stripped, as an action code with bounded
metadata per #3412.

**Pipeline.** Detect the type from magic bytes, never from the extension or the server's
`Content-Type`; reject anything not on the accept list; sanitise; hash; store; record. Accept list
for the MVP: PDF, plain text and Markdown. HTML is stored but never served from our origin.
Office formats are not accepted — macro-bearing formats (`.doc`, `.xls`, `.docm`) are refused
outright, and `.docx` is not needed since #3419's import scope is text and Markdown. SVG is
refused, being scriptable.

**What is stripped from a PDF:** JavaScript (`/JS`, `/JavaScript`), document and annotation
actions (`/OpenAction`, `/AA`), `/Launch`, `/EmbeddedFile`, `/RichMedia`, `/Movie`, `/Sound`,
XFA form definitions, and remote-go-to actions. Plain link annotations (`/URI`) are kept: they are
a phishing vector rather than a code-execution one, stripping them would break real references in
minutes, and the safe-serving rules below plus the visible provenance link cover the residual
risk. A structural tool that removes objects deterministically (qpdf, or pikepdf on top of it) is
preferred over a full re-render; a Ghostscript rewrite is a stronger flattening option but can
destroy tagged-PDF structure that screen readers depend on, which conflicts with #3415, so it is
not the default.

**Safe serving, because stripping is never perfect.** Documents are served with
`Content-Disposition: attachment` and `X-Content-Type-Options: nosniff`, never rendered inline in
the application origin. #3435's design already helps here: downloads go through the authorization
layer to short-lived signed object-storage URLs on a different host, so a malicious file cannot
reach the app's session context. Emails link to documents and never attach them, which removes
the mail-attachment vector entirely and helps deliverability.

**Failure is quarantine, not pass-through.** A file that cannot be parsed, or whose sanitisation
fails, is quarantined and flagged for review rather than stored as servable. For a school
submission the school is still created — a real FAU must not be blocked — and the reviewer sees
the failure.

Malware scanning is a separate concern from active-content stripping and is not covered by it;
whether to add a scanner to the pipeline is an open item rather than a decision here.

## Resolution and redirect semantics

The register holds slug history: `slug pair → entity UUID`, with a validity period. Resolution
rules, in order:

1. Pair matches the current holder → serve.
2. Pair matches exactly one historical holder, and the pair has not been re-issued → 301 to that
   entity's current canonical path.
3. Pair has been re-issued to a different entity → the current holder wins; the prior holder
   remains reachable at its `/s/<uuid>` path.
4. No match → 404. Never guess.

## Error handling

| Case | Response |
|---|---|
| Unknown municipality pair | 404, offering `/fau` browse |
| Known pair, unknown school | 404 scoped to that municipality page, which lists its schools |
| Retired pair or slug | 301 to the current canonical path |
| Closed school | 410 with a pointer to the municipality page, so FAU history does not vanish silently |
| Unclaimed school page | 200, `noindex`, "har ikke FAU her ennå", plus signup |
| Unverified school | 200 at `/s/<uuid>` only; no pretty slug yet, `noindex` |
| Decision link unfetchable | School still created; the record carries the failed status and the reviewer sees it |
| `/s/<uuid>` after a merger | 301 to the successor's canonical path |
| Workspace resource in another FAU | 404, never 403 |
| Mixed case or trailing slash | 301 to the lowercase, slash-free canonical |

## Indexing

Claimed school pages are indexable. Unclaimed and unverified pages are `noindex`, so search sees
only real FAU-er rather than thousands of near-identical pages, which is both a doorway-page risk
and a claim about schools that have not chosen to be listed. Every page declares a canonical URL
pointing at its pretty path when it has one, and at `/s/<uuid>` when it does not.

## Testing

- Slug-history resolution against the real cases: 0706→0710 Sandefjord, 0716 Våle→Re, Herøy in
  two counties, and a renumbering such as a 38xx→39xx county change.
- Transliteration round-trips for æ, ø, å in both municipality and school names.
- Root collisions are structurally impossible and a test should pin that: every generated path
  lives under `/fau/`, `/s/` or `/bli-med/`, municipality segments always begin with digits and
  contain a hyphen, and school segments are nested. A test asserting that no register-generated
  segment is ever emitted at the root protects the property against a later "prettier URLs"
  refactor. The reserved list therefore constrains only the pages we author ourselves.
- `/bli-med/<uuid>` grants no access on its own, and forwarding it confers nothing.
- The fetcher's host matching accepts `udir.no`, `www.udir.no` and `baerum.kommune.no`, and
  refuses `kommune.no.attacker.example`, `evil-kommune.no.attacker.example`,
  `udir.no@attacker.example` and a punycode variant of an allowlisted host that is not that host.
- The fetcher refuses loopback, link-local and private addresses even for an allowlisted host,
  including via a redirect chain, and refuses a redirect that leaves the allowlist.
- A non-allowlisted URL is stored with the submission and never fetched, and the reviewer sees it
  as pending manual retrieval.
- A school submission sends exactly one notification, is rate limited, and escapes submitted text
  in the email.
- Ingest strips active content: a PDF carrying JavaScript, an `/OpenAction`, a `/Launch`, an
  embedded file and an XFA form comes out with none of them, and the original is retained
  quarantined with its hash intact.
- Type detection uses magic bytes: a `.pdf` extension on a ZIP, and an HTML file served as
  `application/pdf`, are both refused.
- A file that fails sanitisation is quarantined rather than served, and the school submission
  still completes.
- A workspace link into a non-member FAU returns 404, and one into a member FAU switches context
  visibly.
- Case and trailing-slash canonicalisation.

## What this changes elsewhere

- **ADR-001**: adds `/fau`, `/s` and `/bli-med` to the root paths it reserves, and records that
  no user- or register-derived segment is ever served from the root;
  fixes UUIDs as the identifier form in URLs; confirms error codes over display text remains
  necessary (#3439).
- **#3441**: the register owns slug generation, slug history, the reserved-word list, and
  decision-document provenance per school. Its outreach links use `/bli-med/<uuid>`.
- **#3413 / #3417**: signup accepts a prefilled school from either entry point; the prefill is a
  correctable suggestion, never verified fact, and grants nothing.
- **#3422 / #3423**: routing must keep an outer locale prefix cheap to add; school pages are real
  indexable pages with their own titles, not internal routes.
- **#3412**: `id` becomes a UUID for entities reachable by URL; composite tenant keys unchanged.
- **#3426**: retention of decision-document links, archived copies and quarantined originals, and
  the SSRF constraints on the fetcher.
- **#3419 / #3435**: the same ingest pipeline governs FAU uploads, not only fetched documents;
  #3435's signed-URL downloads from a separate host are part of what makes serving safe.
- **#3410**: one more transactional message type, the school-submission notification to our
  operational address.

## Open items

- Retention period for decision links and their archived copies (#3426), and whether the archived
  copy is ever shown publicly rather than only to reviewers.
- Whether a malware scanner joins the ingest pipeline alongside active-content stripping, and
  where it runs.
- Retention of quarantined originals, which are kept for provenance and never served (#3426).
- UUIDv7 versus v4, if the timestamp exposure is unwanted.
- Who reviews the unverified-school queue, and the target turnaround.
- The county-suffix form is no longer needed for municipalities, since the number-name pair
  disambiguates; it remains available if a school-level collision ever needs it.
