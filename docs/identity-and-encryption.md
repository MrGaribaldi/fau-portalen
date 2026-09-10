# ADR-003: Identity, encryption at rest, and the recovery contact

Status: proposed, 10 September 2026. Amends ADR-001 (docs/repo-container-contract.md) by moving
authentication to an external provider, and builds on the tenant and role model approved on #3412
(docs/tenant-role-history-design.md) without changing it. Decisions here were taken with Erik in
conversation on 10 September 2026 and are logged in docs/planning-decisions.md.

## Why this document exists

Two requests arrived together and only look like one: buy authentication instead of building it,
and encrypt user data so a leak is not a disclosure. They interact, but not in the way the initial
framing suggested, and the difference is the whole content of this document.

An external magic-link provider does not give users encryption keys. Magic links prove control of
a mailbox; there is no secret the user knows or holds, so there is nothing to derive a key from.
Whoever can complete an email challenge can be granted access by the server, which means the
server must be able to reach the data. Moving authentication to a provider changes who checks the
email. It does not remove our ability to decrypt.

The recovery requirement settles it in the same direction. FAU exists because committees turn over
every year and the archive must survive. Erik's decision on 10 September 2026: recovery must work
even when no member remains. A design where only members hold keys cannot do that - a committee
that vanishes without handing over would destroy the archive permanently, which is the exact
failure the product is sold to prevent.

So we hold the keys. Everything below follows from stating that plainly instead of implying
otherwise.

## The trust model, stated in the words we will use publicly

Encryption here defends against a **leak**, not against us.

- A stolen database, a stolen backup, or a stolen object-storage bucket is inert on its own. The
  keys are in none of them.
- Reading one FAU's documents means driving the live application: seat a member, pass magic-link
  verification, download. Then do it again for the next FAU. No step yields more than one FAU.
- Every one of those steps is audited and notifies that FAU's members, so bulk exfiltration is
  slow, linear in the number of FAU-er, and loud.
- Root on a cluster node defeats all of it. This document says so rather than implying a
  guarantee we cannot make.

**We never write "we cannot see your data."** Not in the DPA, not in privacy text, not in sales
material. The accurate phrase is restricted and audited access. Claiming zero access while holding
a key path is the kind of statement that turns an incident into a misrepresentation.

## Decisions

### 1. Authentication is bought. Zitadel Cloud, Swiss region, for now

Confirms the preference recorded on 7 September 2026 ("magic links, no stored passwords; Zitadel
or a similar service is a research candidate"). Zitadel Cloud in the Swiss region holds
credentials and runs the magic-link flow. We do not operate an identity provider: one we run is
one we patch, upgrade and get paged for, which is the reason to buy rather than build.

**This is a time-boxed exception to the European ownership rule, not a clean fit.** The Swiss
entity is a subsidiary of Zitadel LLC, registered in California. Authentication is
customer-facing, so the rule recorded for supplier ownership - customer-facing services must be
European - is not satisfied by Swiss hosting under a US parent. Two facts limit the exposure and
justify proceeding: the provider holds email addresses and authentication events, never document
content, because content never leaves our cluster encrypted under keys the provider has no access
to; and passwordless authentication makes the provider unusually cheap to replace, since there are
no password hashes to migrate.

The exception is accepted on the condition in decision 3, and belongs in the #3409 supplier
mapping as a customer-facing processor with a DPA.

**The European target is unidentified, and that is the honest state.** The first draft named Ory
as a German-owned fallback; it is US-owned, and the entry was wrong. The correction is recorded
rather than deleted, because "it sounds European" is exactly the reasoning the ownership rule
exists to stop. No replacement has been identified yet. Finding one is open item 2, and until it
is answered the exit in decision 3 is a capability we are building rather than a route we have
surveyed.

That ordering is deliberate. The portability rules below cost little if applied from the start and
are close to unaffordable if retrofitted, so they do not wait on knowing the destination.

### 2. Zitadel authenticates. We authorize. Always

The provider answers exactly one question: is this person in control of this mailbox. Every
question about what they may do is answered by our own database, per operation, using the model in
docs/tenant-role-history-design.md. No roles, no groups, no permissions, and no tenant structure
are stored in the identity provider.

This is a security boundary and a portability boundary at once, and it is not negotiable in
implementation. The moment an authorization decision reads a claim minted by the provider, both
properties are lost.

### 3. The identity provider is replaceable by design

Required because of decision 1's US parent, and cheap to honour if done from the start. These
rules are binding on implementation, not aspirations. Several were sharpened by a review from
Codex on 10 September 2026.

**Identity and mapping**

- Every person has an **internal UUID**, generated by us. The provider's `sub` is never a primary
  key, never a foreign key, and never appears in application tables other than the mapping.
- Identities map as **(issuer, subject) → internal user id**. Issuer, not a provider nickname, so
  the row stays meaningful when a provider changes hostname or region.
- An internal user may hold **several identity mappings at once**. This is what allows two
  providers to run concurrently during a migration instead of requiring a cutover, and it is the
  single rule most expensive to add later.

**What never lives in the provider**

- FAU-er, memberships, roles, recovery-contact seats, and any authorization that gates a key
  unwrap live in our database. Not in provider organisations, not in vendor claims, not in groups.
- Authoritative audit logs are ours. A provider's own log is corroboration, never the record.

**Protocol and abstraction**

- Authentication uses standard **OIDC Authorization Code Flow with PKCE**, and nothing else.
- Issuer, endpoints, client identifiers and claim mappings are configuration, never constants in
  code.
- `acr`, `amr`, `auth_time` and the notion of recent re-authentication reach the application
  through our own abstraction. Decision 4's step-up must be expressed in our terms, because every
  provider interprets these differently and coding against one interpretation is a silent lock-in.
- Provider-specific management APIs sit behind one thin adapter with a documented interface and no
  other caller.
- Session and refresh semantics are ours, so session policy is not provider-locked.

**Staying ready to leave**

- The user-to-internal-id mapping and the minimum profile needed to re-enrol are exported on a
  schedule. An exit that depends on the provider's API still working on the day we leave is not an
  exit.

**Acceptance test for the boundary:** the migration to a second provider must be describable as
configuration plus a re-mapping job, with no change to authorization code.

### 3a. Migration runs as an overlap, not a cutover

Credential material does not move. Password hashes, TOTP secrets and passkeys are not portable
between providers, so a migration re-enrols rather than transfers. The sequence:

1. Add the new provider as a **second accepted issuer**; both are live.
2. Existing members continue to authenticate through the old one.
3. A controlled enrolment flow moves members to the new provider as they appear.
4. The new identity is **linked to the existing internal user** - a second mapping row, not a new
   account. Memberships, roles and encrypted content are untouched, because none of them ever
   referenced the provider.
5. Any second factor is re-enrolled, since it cannot be carried across.
6. The old provider is disabled only after the migration window closes.

Decision 4a lowers the cost of this considerably: with passwordless email authentication there are
no password hashes in the first place, so ordinary members re-enrol by doing nothing more than
logging in. The corollary is worth stating - **the day we adopt passkeys, migration stops being
free**, because passkeys are bound to the provider's relying-party identity and every member must
re-enrol deliberately. That is a reason to weigh passkey adoption against the exit we are trying
to preserve, not a reason to avoid it.

### 4. Long sessions, step-up for privileged actions

Sessions stay valid for 30 days for ordinary reading and editing. Privileged actions - inviting
members, granting or revoking roles, and anything a recovery contact initiates - require step-up
re-authentication regardless of session age. This satisfies prosjektgrunnlag.md section 13's
two-factor requirement for administrative roles without forcing a second factor on a parent who
only wants to read the minutes. A shared-device option opts out of the long session entirely.

Long sessions are safe here only because the approved tenant model re-evaluates authorization on
every operation rather than at login. A revoked role dies immediately inside a live session.

Two billing rules follow from Zitadel counting a daily active user as anyone who authenticates
**or refreshes a token** on a given day:

- Tokens are refreshed on real user interaction only. No background refresh, no keep-alive on an
  idle tab, no service worker that renews while nobody is using the app. Any of those manufacture
  billable activity for members who are not there.
- Machine-to-machine accounts count as daily active users too. One service account, not one per
  service.

### 4a. The requirement is passwordless email authentication, not the magic link specifically

Recorded because the distinction turned out to constrain provider choice, which the first draft
did not notice.

What the product needs is that a member proves control of their mailbox and never holds a
password. Two mechanisms satisfy that equally: a clickable link, or a short one-time code emailed
and typed into the page. The architecture must state the requirement at that level, because the
mechanisms are not equally available across providers - Zitadel offers magic links natively, while
Keycloak offers neither without a community extension.

There is also a practical argument for codes that is independent of provider choice. Mail security
products fetch links in incoming messages to scan them, and a scanner that follows a single-use
magic link consumes it before the recipient clicks. School and municipal mail systems are exactly
the environment where that happens, and it presents to the user as "the link says it has already
been used". An emailed code has no equivalent failure.

The mechanism is therefore an implementation choice per provider, listed as open item 3, and the
identity abstraction in decision 3 must not leak either mechanism into the application.

### 5. Key hierarchy, and no bulk decryption

Three tiers:

1. Each FAU has a **data key**. All protected content is encrypted under it.
2. That data key is stored **wrapped**, on the tenant row, next to the data it protects.
3. The **master key** that unwraps it exists only inside a small key service, in its own
   namespace, holding it in memory and never writing it beside the data.

The key service exposes exactly one operation: unwrap this one FAU's data key, for this
authenticated session. It logs every call and rate-limits them. It has no endpoint that returns
more than one FAU's key, and no endpoint that returns the master key. The backend never holds the
master key.

The property this buys is the one that matters: a compromised backend cannot decrypt everything in
one pass. It must ask, once per FAU, in the open, at a rate the service controls. Mass decryption
then looks like mass decryption in the logs rather than like one query.

The key service must stay small enough to review in an afternoon. If it grows features, it stops
being a boundary.

### 6. What is encrypted, and what is not

**Encrypted under the FAU data key:** document bodies, change sets, uploaded object bytes,
document titles, and original filenames.

Titles and filenames are inside the boundary deliberately. "Klage på lærer Hansen" or
`bekymringsmelding-elev.pdf` discloses as much as the file it names, and a title sitting in
plaintext sits in every backup.

**Plaintext, because the product cannot work otherwise:** member emails and names, roles and their
date ranges, FAU and school names, document identifiers, sizes, hashes and content types,
timestamps, and audit event types.

Consequences accepted with the boundary: there is no cross-FAU search, by construction, including
for us. Full-text search within one FAU requires decryption in an authorized session, and whether
the MVP has search at all is undecided. docs/storage-security-proposal.md is otherwise unchanged -
server-generated object keys, short-lived signed uploads, backend-proxied downloads - with
encryption applied before bytes reach object storage, so Hetzner stores ciphertext.

### 7. Deletion becomes provable

Destroying an FAU's wrapped data key makes its content unrecoverable, including in every backup
already written. This turns "fullstendig sletting ved avsluttet kundeforhold" in
prosjektgrunnlag.md section 13 from a promise about deletion jobs into a cryptographic fact.

Key destruction must therefore be as carefully guarded as key use: two-step, audited, and
irreversible by design. A mistaken key destruction is indistinguishable from data loss.

### 8. The recovery contact

Replaces the earlier idea of the operator temporarily holding an admin role. Each FAU chooses its
recovery contact: **us, or a school representative**.

A recovery contact **cannot**:

- read documents, change sets or uploaded files;
- read the audit log;
- grant itself membership or any role in the FAU.

A recovery contact **can**, after step-up authentication, do exactly one thing: **initiate the
addition of a new member** to the FAU it is attached to.

This is why the design works without an exception to the model approved on #3412: because we hold
the keys, granting membership is pure authorization. Nobody needs content access in order to
restore someone else's content access. The rule that handover-only roles get no document access
stands unmodified, and applies to us as well.

Residual risk, accepted rather than engineered away: a malicious recovery contact can deliberately
cause an unauthorised person to be seated, and that person will have access until the misuse is
noticed and acted on. Continuity has this cost. The notification rules in decision 10 are what
make the window short rather than silent.

Once school representatives are established as recovery contacts, we remove ourselves from that
seat for those FAU-er. Being the recovery contact for every FAU in the country is a concentration
of trust worth dismantling as soon as there is somewhere to move it.

Vacating the seat does not vacate the oversight. We remain the notified second party for every
recovery on every FAU, whether or not we hold the seat - see decision 10. Losing the seat means
losing the ability to initiate, not the ability to see.

### 9. Verifying a school representative

A nominated representative must be the principal or the inspector at the school the FAU belongs
to. No public register reliably exposes who that is, so verification is assembled from layers:

1. **The FAU nominates**, while it still has members. The nomination is itself evidence, and it
   happens long before any recovery.
2. **Domain-verified email.** The school is looked up in the national school register to obtain
   its official domain; the nominee verifies a magic link at an address on that school or
   municipality domain. This proves affiliation, not title.
3. **Recorded manual title check.** We confirm the title against the school's or municipality's
   published staff listing, or by telephone, and record what was checked, by whom, and on what
   date.

Step 3 does not scale to a national roll-out and is not meant to. It is affordable while
onboarding is hands-on, and it is the step to automate or replace when that stops being true.

Because a recovery contact cannot read content, a school representative in that seat does not
become a processor of FAU document data. That keeps the school out of the data-processing chain
and makes the conversation with a municipality substantially easier.

### 10. Notification and audit

Every membership change initiated by a recovery contact produces:

- a **permanent audit entry**, which no operator role can edit or delete;
- an **email to every current member** of that FAU, sent when the invitation is **created** and
  again when it is **accepted** - two events, because the window between them is where misuse
  would live;
- an **in-app banner** at login, shown for 14 days from the event.

The banner expires after 14 days whether or not a given member has logged in. The email is what
reaches members who do not log in, and the audit entry is permanent regardless.

Legal context for the empty case, from Erik on 10 September 2026: a public school is required to
have an active FAU, so an FAU with no members at all is an anomaly rather than a normal end state.
The rules below are still specified and still tested - an anomaly that cannot happen is exactly
the one nobody has written code for - but they describe a rare path, not the yearly cycle.

**When the FAU has no remaining members** - the case the recovery contact exists for - notification
goes to two places: the addresses that most recently held a valid role before it lapsed, and the
**second party** - the recovery option that did not initiate the action. If a school
representative initiates, we are notified; if we initiate, the seated school representative is.

We are the second party for every FAU, including those where a school representative holds the
seat and we do not. This is oversight without capability: it carries no power to initiate, read
content or read audit, only the right to be told. Two independent parties therefore see every
recovery, so no single actor can run a silent one.

This requires retaining lapsed members' email addresses after their role ends, for the sole
purpose of notifying them about a recovery. That is personal data kept past the relationship. It
needs a stated retention period - 24 months proposed - and it belongs in the DPA explicitly rather
than being implemented quietly.

## What this changes elsewhere

- **#3412 / docs/tenant-role-history-design.md**: unchanged. The recovery contact is a new role
  that respects the existing rule rather than an exception to it. Authentication moves out; the
  authorization model stays exactly as approved.
- **docs/storage-security-proposal.md**: unchanged in flow. Adds that bytes are encrypted under
  the FAU data key before upload, so stored objects are ciphertext, and that the negative-test
  list gains key-boundary cases.
- **#3409 supplier mapping**: gains Zitadel as a customer-facing processor, Swiss-hosted with a US
  parent, recorded as a time-boxed exception with decision 3 as its exit condition. Needs a DPA.
- **#3410 transactional email**: recovery notifications are transactional mail to people who may
  no longer be members, which the provider evaluation should account for.
- **prosjektgrunnlag.md section 13**: two-factor for administrative roles is satisfied by
  decision 4's step-up rather than by blanket MFA. Deletion is satisfied by decision 7.
- **ADR-001**: authentication is no longer ours to serve; the reserved path set gains whatever the
  OIDC callback needs.

## Testing

These are requirements for later integration tests, in the spirit of the #3412 table:

| Scenario | Expected result |
| --- | --- |
| Compromised backend requests many FAU keys in sequence | Rate limit trips; each unwrap individually audited; no endpoint returns more than one key |
| Database and object-storage dump taken without the key service | No plaintext content, titles or filenames recoverable |
| Recovery contact attempts to read a document or the audit log | Denied; no content leaked in the error |
| Recovery contact attempts to grant itself membership | Denied |
| Recovery contact initiates an addition without step-up re-auth | Denied |
| Invitation created by recovery contact, then accepted | Two notifications to every current member; two permanent audit entries |
| Recovery on an FAU with no valid members | Last known member addresses notified; the non-initiating anchor notified |
| Nominated school representative verifies at a non-school domain | Rejected; nomination stays pending |
| Role revoked mid-session inside a 30-day session | Next operation denied; no reliance on session age |
| Authorization decision derived from a provider-minted claim | Must not exist; caught by review and by test |
| Provider `sub` used as a key in any application table other than the mapping | Must not exist; caught by schema review |
| One internal user given a second (issuer, subject) mapping | Both identities resolve to the same internal user; memberships, roles and content unchanged |
| Two issuers accepted simultaneously | Members authenticate through either; no duplicate accounts created |
| Authentication attempted without PKCE | Rejected |
| FAU key destroyed, then a restore of an old backup attempted | Content unrecoverable |

## Open items

1. **Zitadel free-tier metric.** The pricing page says "100 Daily Active Users" for free and
   "25'000 Daily Active Users per month included" for Pro. Those phrasings imply different
   metrics, and the difference is large. Under the stricter reading - 100 activity units summed
   across the month - three people active every calendar day consume about 90 of them, so the free
   tier is a **development tier, not a pilot tier**, and a single machine account spends roughly
   30 by itself. Confirm in writing before any pricing claim depends on it, and monitor service
   account activity rather than assuming it is free.
2. **A European provider has not been found.** Ory is US-owned; nothing else has been identified
   that satisfies the ownership rule end to end. This needs its own search, evaluated on
   ownership, on whether passwordless email authentication is available without a
   community-maintained plugin, and on total cost at 10, 100 and 1000 FAU-er. Until it is
   answered, decision 1's exception has a condition but not a destination.
3. **Whether the requirement is "magic link" or "passwordless by email".** See decision 4a.
4. **Search.** Whether the MVP has per-FAU full-text search, and if so whether it decrypts in the
   session or maintains a per-FAU encrypted index.
5. **Retention of lapsed member addresses.** 24 months proposed; needs a decision and a DPA clause.
6. **Key service failure modes.** What happens to a live session when the key service is
   unavailable, and whether an FAU key is cached in the backend for the life of a session or
   fetched per operation. The first is faster; the second is stricter.
7. **Recovery contact for FAU-er that choose neither.** Whether that is permitted at all, and what
   the product does when such an FAU loses its last member.
