# ADR-003: Identity, encryption at rest, and the recovery contact

Status: proposed. Written 10 September 2026. The identity provider has been decided three times:
Zitadel Cloud on 10 September, PropelAuth on 21 September, and **Hanko on 22 September**, which is
what this document now describes. Provider-specific facts are confined to decision 1 so a fourth
change costs one edit. The administrative-MFA decision was settled 21 September.

Amends ADR-001 (docs/repo-container-contract.md) by moving authentication to an external provider,
and builds on the tenant and role model approved on #3412 (docs/tenant-role-history-design.md)
without changing it. Every decision here is logged in docs/planning-decisions.md.

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

### What we defend against, and what we do not claim

Boundary set by Erik, 22 September 2026.

> We defend against mistakes and against casual misuse by people who have legitimate access. We do
> not claim to withstand a determined attacker who is already inside, and we never describe the
> product as secure against one.

This is the same discipline as the rule above against writing "we cannot see your data": do not
claim a property we cannot back.

It is a scoping statement rather than a licence to skip work, and it cuts both ways. It *justifies*
server-side sanitisation of imported, pasted and published content, because the realistic case is
an unaware parent forwarding or pasting something they never inspected - which is far more likely
than a crafted attack and does just as much harm to whoever opens it. It *scopes down* adversarial
hardening against authenticated members: payload caps and rate limits, not fuzzing of malformed
editor traffic. And it leaves the main case unchanged, because the realistic malicious member is a
compromised honest one, which the second-factor gate in decision 4 and the notification rules in
decision 10 already address.

### Why not end-to-end encryption

Asked and answered 22 September 2026, recorded here because it is the obvious question and will be
asked again.

End-to-end encryption would replace the sentence above with a genuine "we cannot read it". It was
rejected for three reasons, in descending order of how decisive they are.

**Continuity is the product, and E2E is incompatible with it.** FAU exists so an archive outlives
every member leaving. Under real end-to-end encryption, when the last key-holder goes, the data is
gone. Decision 8's recovery contact is an escrow mechanism, and escrow is exactly what makes a
system not end-to-end. Continuity across total turnover or E2E - not both.

**It would remove protection against the likelier attacker.** Server-side validation of the
document JSON (#3490) and the entire ingest pipeline (#3447 - malware scanning, active-content
stripping, conversion) all require the server to read the content. Under E2E they move into the
browser or vanish, which trades defence against a compromised or malicious *member* for defence
against ourselves. For a parents' council, a bad upload reaching other parents' browsers is the
more probable event, and "every client validates correctly" is a weaker guarantee than one Rust
validator.

**There is nothing to derive a key from.** Passwordless login proves mailbox control and yields no
secret. The alternatives are a password, reversing a core product decision; a separate passphrase,
which is the second secret the product was designed to avoid; or passkey PRF, which is ruled out
for the MVP, unevenly supported across authenticators, and which the WebAuthn specification's own
co-editors warn against for encryption precisely because the data dies with the passkey.

There is also an honesty problem at the end of it. We serve the JavaScript and we hold the member
key directory, so it would be E2E against a passive server only - genuinely end-to-end only if
members verified each other's keys out of band, which volunteer parents will not do. We would have
built it and still not been able to write the sentence.

**A partial version was considered and also dropped.** A "sensitive document" class, client
encrypted to current members, was worth weighing while it looked as though FAU would hold reports
about named individuals. Erik's scope boundary below removes that case, and what remains - internal
discussion, draft positions, matters involving a named child raised in a meeting - is already
covered by encrypting bodies, titles and filenames under decision 6. So the document envelope needs
no encrypted-to-members variant, and #3490 stays simpler for it.

### What this product is not: a reporting channel

Scope boundary set by Erik, 22 September 2026, recorded so it is not reintroduced by drift.

**FAU will never carry a whistleblowing or reporting channel.** An FAU should not be involved in
reporting matters concerning the school, and a complaint against an FAU member belongs with the
school rather than with us. This is a boundary about what the organisation is for, not a feature
deferred for cost.

Two things follow. Nothing in the roadmap (#3433) should reintroduce varsling, intake of reports
about named individuals, or a confidential channel between a parent and anyone outside the FAU.
And the design does not need the protections such a channel would demand - which is what closes the
end-to-end question above rather than merely postponing it.

## Decisions

### 1. Authentication is bought. Hanko, decided 22 September 2026

Confirms the preference recorded on 7 September 2026 - passwordless by email, no stored passwords.
Hanko holds credentials and runs the authentication flow. We do not operate an identity provider:
one we run is one we patch, upgrade and get paged for, which is the reason to buy rather than
build.

This is the third provider named in this document, so the facts that decide it are kept in one
table and the rest of the ADR refers to "the provider". A fourth change should cost an edit here
and nothing else.

| | Hanko |
| --- | --- |
| Entity | Hanko GmbH, Ringstraße 19, Kiel, Germany |
| Ownership | German. Satisfies the European rule for customer-facing services rather than excepting it |
| Data location | EU. Infrastructure subprocessors are AWS, Hetzner and adesso as a service |
| Data protection | Offers a Hanko Cloud DPA. GDPR is the default operating model, not a bolt-on; no Article 44 transfer to justify |
| Certification | ISO 27001 in progress, per Hanko. Not independently verified here |
| Cost | Free to 10,000 monthly active users, then USD 0.01/MAU; a startup programme offers 1M MAU free |
| Methods | Passkeys, six-digit email passcodes, passwords, social, SSO; MFA by TOTP app or FIDO security key |
| Exit | Open source and self-hostable. The provider can be replaced by running it ourselves |

**What this supersedes.** PropelAuth, named on 21 September, is dropped: it does not offer a DPA
on the Free tier, which is the tier the decision assumed. That is disqualifying rather than
inconvenient - a processor handling personal data without an Article 28 contract is not a
supplier we can lawfully use, whatever its other merits. Before it, Zitadel Cloud. Both are
recorded rather than erased, because the reasoning that failed is worth keeping.

**What the change gives back.** The European ownership rule is satisfied rather than excepted, so
FAU no longer carries a customer-facing exception for identity. Login mail no longer leaves the
EU, which retires the 21 September decision to accept US-hosted authentication mail "for now".
And because Hanko is open source and self-hostable, decision 3's exit stops being a design
discipline pointing at nothing and becomes a route we could actually take.

**The one qualification, recorded honestly.** Hanko's infrastructure subprocessors include AWS,
which is US-owned even when the region is in the EU, so the chain still reaches a US company. The
difference from PropelAuth is real and worth stating precisely rather than waving away: the
controller is German, the data stays in the EU, and there is no transfer to the United States
that needs a legal basis. Erik accepted this on 22 September on the grounds that Hanko documents
the arrangement. Hanko also lists Hetzner and adesso as infrastructure, so AWS may not be the only
path. "Not a problem right now" is the right description - revisit it if Hanko's hosting changes,
or if US access to EU-resident data becomes a live issue rather than a theoretical one.

This belongs in the #3409 supplier mapping as a customer-facing processor in Germany, with a
signed DPA, and with AWS recorded as its infrastructure subprocessor.

### 2. The provider authenticates. We authorize. Always

The provider answers exactly one question: is this person in control of this mailbox. Every
question about what they may do is answered by our own database, per operation, using the model in
docs/tenant-role-history-design.md. No roles, no permissions and no role assignments are stored in
the identity provider, and no authorization decision ever reads a claim it minted.

**What Hanko is given: an email address. Nothing else.** The data-minimisation rule Erik set on
21 September - no more information than needed, and specifically no names - now holds structurally
rather than by discipline. Hanko has no B2B organisation model to mirror membership into; its
multi-tenancy isolates whole user pools per deployment rather than grouping members inside one.
So there is no FAU membership in the provider at all.

- **Given:** the email address, and the authentication material the user creates - passkey
  credentials, a TOTP secret.
- **Not given:** names, FAU membership, roles, permissions, document content, titles, filenames,
  or anything derived from them.

This is strictly better than the previous design, where a US processor would have learned that an
account was attached to a particular organisation - personal data whenever the organisation
identifies a school. That exposure is now gone rather than minimised, and the question of whether
to name organisations opaquely is moot.

This is a security boundary and a portability boundary at once, and it is not negotiable in
implementation. The moment an authorization decision reads a claim minted by the provider, both
properties are lost.

### 3. The identity provider is replaceable by design

Required regardless of which provider decision 1 names, and cheap to honour if done from the
start. These
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

### 4. Long sessions, and an MFA gate for privileged actions

Sessions stay valid for 30 days for ordinary reading and editing. Privileged actions - inviting
members, granting or revoking roles, and anything a recovery contact initiates - are gated on two
server-side checks, decided by Erik on #3414 and recorded 21 September 2026:

1. **The account has a second factor enrolled.** Hanko's user object carries `totp_enabled`,
   `auth_app_set_up` and `security_keys_enabled`, so the check reads the state directly and can
   distinguish a TOTP app from a FIDO security key. An account without either is refused the
   action and sent to MFA setup, not merely warned.
2. **The session is fresh.** Logged in within the last 30-60 minutes. A stale session is forced
   back through login, which re-triggers TOTP.

A shared-device option opts out of the long session entirely.

**Stated precisely, because the first draft overclaimed it.** This is two independent factors at
login - control of a mailbox, plus a TOTP secret on a device - with a recency bound on their use.
It is *not* per-action step-up MFA: there is no challenge bound to the specific operation and no
action-scoped one-time grant, so it does not defend against an attacker already inside a fresh,
MFA'd session. Erik accepted that residual risk on #3414 for this threat model - volunteer
administrators on shared or forgotten devices.

Hanko changes one input here, in our favour. It is passkey-first, and a passkey is already
possession plus a local biometric or PIN. An administrator who signs in with a passkey has
therefore met a stronger bar at login than one who types an emailed code and then a TOTP digit,
so the guard should accept a passkey as satisfying check 1 rather than demanding TOTP on top of
it. Worth settling with the endpoint list rather than now.

This is what satisfies prosjektgrunnlag.md section 13's two-factor requirement for administrative
roles, without forcing a second factor on a parent who only wants to read the minutes. Codex's
review of 10 September was correct that the earlier formulation - repeating the email factor and
calling it step-up - did not establish two factors. TOTP is what closes that gap.

**How "enforce MFA" is implemented, clarified by Erik on 21 September 2026.** It means the two
server-side checks above - a second factor enrolled, and the last authentication recent enough -
and not a provider-side "require 2FA" organisation setting. Enforcement belongs on our side of the
boundary, which is where decision 2 puts every authorization decision. That reasoning was written
when the provider was PropelAuth and it survives the change unaltered, which is a sign it was the
right reason.

Still to define before the guard ships: the explicit list of sensitive endpoints it covers, and
the freshness threshold within the 30-60 minute range.

Long sessions are safe here only because the approved tenant model re-evaluates authorization on
every operation rather than at login. A revoked role dies immediately inside a live session.

Cost rules, now that the provider bills monthly rather than daily active users:

- The free tier covers 10,000 monthly active users, above any realistic MVP or pilot scale, and
  usage beyond it is USD 0.01 per monthly active user. Hanko also runs a startup programme
  offering 1M MAU free, worth applying for. The pricing metric no longer constrains session design
  the way Zitadel's daily-active-user counting did.
- Machine-to-machine accounts still count as active users where they authenticate. One service
  account, not one per service.

### 4a. The requirement is passwordless email authentication, not the magic link specifically

Recorded because the distinction turned out to constrain provider choice, which the first draft
did not notice.

What the product needs is that a member proves control of their mailbox and never holds a
password. Two mechanisms satisfy that equally: a clickable link, or a short one-time code emailed
and typed into the page. The architecture must state the requirement at that level, because the
mechanisms are not equally available across providers - Hanko sends a six-digit email passcode
natively, while Keycloak offers neither without a community extension.

There is also a practical argument for codes that is independent of provider choice. Mail security
products fetch links in incoming messages to scan them, and a scanner that follows a single-use
magic link consumes it before the recipient clicks. School and municipal mail systems are exactly
the environment where that happens, and it presents to the user as "the link says it has already
been used". An emailed code has no equivalent failure.

The provider change of 22 September settles this in passing, and retires a decision taken the day
before. Hanko's native mechanism is a six-digit email passcode, which is the mechanism this
section already argued for: mail security products at schools and municipalities fetch links to
scan them, and a scanner that follows a single-use magic link consumes it before the parent
clicks, presenting as "this link has already been used". A passcode has no equivalent failure.
Passkeys are the primary method and avoid the mail path entirely for anyone who enrols one.

The 21 September decision to accept US-hosted authentication mail from PropelAuth via Postmark,
revisited when revenue allowed, is **superseded and no longer needed**: Hanko sends the passcode
itself from EU infrastructure. So the European transactional-mail decision on #3410 no longer has
a customer-facing gap to account for.

The mechanism therefore stays an implementation choice, and the identity abstraction in decision 3
must not leak it into the application - which matters more now that two mechanisms ship side by
side. Whether passkeys are offered in the MVP at all is open item 3.

### 5. Key hierarchy, and no bulk decryption

Revised 22 September 2026. The first version wrapped every FAU's data key under one global master
key, which made decision 7's deletion claim false - see that section. The tier that changes is the
middle one: the wrapping key is now **per FAU**, and it never touches the application database.

Revised again 22 September 2026 to add a per-document tier, which is what makes purging a single
document provable rather than best-effort. See decision 7a.

1. **Each document has a data key.** That document's content - step batches, checkpoints, title,
   comment bodies, attached bytes and filenames - is encrypted under it.
2. **Document keys live only in the key service's datastore.** They are never written to the
   document row and never enter the application database in any form. This is the correction that
   the per-FAU tier taught: a key stored beside the data it protects is a key that survives in
   every backup of that data.
3. **Each FAU has a key-encryption key**, also held only in the key service, which wraps that FAU's
   document keys at rest. Destroying it renders every document key in the FAU unusable in one
   operation, which is what FAU deletion needs.
4. The key service holds a **root key** that encrypts its store at rest, in memory, never written
   beside the data.

The key service exposes exactly one read operation: unwrap **one document's** key, for this
authenticated session. (Amended 24 September 2026: a second, equally narrow read operation exists
for the FAU's inbound private key, see decision 6. The rest of this paragraph applies to it
unchanged.) It logs every call and rate-limits them. It has no endpoint that returns
more than one document key, no endpoint that returns a KEK, and no endpoint that returns the root
key. The backend never holds a KEK or the root key.

Moving the granularity from per-FAU to per-document strengthens the property decision 5 exists for:
a compromised backend must now ask once per *document*, not once per FAU, so reading an FAU's
archive is loud in proportion to its size rather than costing one request.

The property this buys is the one that matters: a compromised backend cannot decrypt everything in
one pass. It must ask, once per FAU, in the open, at a rate the service controls. Mass decryption
then looks like mass decryption in the logs rather than like one query.

The per-FAU tier buys a second property the global master key could not: deletion that survives
into backups, because the key being destroyed was never in the backup to begin with. The
per-document tier extends the same property to one document at a time.

**Where key material must not live.** Not in the application database, which is the point of the
change. Not in a Kubernetes Secret either - a Secret lives in etcd, and etcd is snapshotted to
`fau-k3s-backup`, so a key destroyed in the live cluster would survive in every snapshot. The
same hole one layer down. The key service needs its own datastore with real deletes.

The key service must stay small enough to review in an afternoon. If it grows features, it stops
being a boundary.

### 5a. How long the backend holds a key, and why per-operation fetching is worse

Decided 22 September 2026. Erik's question about live editing is what settled it, and it reversed
the recommendation this document previously carried.

**The workload that breaks per-operation fetching.** Someone edits minutes during a meeting: a few
lines, several minutes of discussion, a few more lines. Autosaves are durable and frequent by
requirement, so each one is a write of an encrypted change set. Fetching the key per operation
means a key-service round trip every few minutes per editor, for hours, across every FAU holding a
meeting on the same Tuesday evening.

**The decisive argument is not performance, it is the audit signal.** Decision 5 exists so that
mass decryption looks like mass decryption in the logs. A log carrying one unwrap per session per
FAU makes an unusual pattern obvious. A log carrying one unwrap per autosave carries tens of
thousands of routine entries a day, and the two hundred malicious ones are invisible inside it.
Per-operation fetching dilutes the very signal the key service was built to produce.

Nor does it buy protection. A compromised backend holds plaintext by construction - it must, to
serve a document to the person reading it - and it can ask per operation just as easily as it can
ask once. The control that matters is the rate and the ceiling, not the interval.

**The rule.** The backend obtains an FAU data key **once per user session per FAU**, and holds it
as a handle in memory only.

- Held for the session, with an idle timeout in the tens of minutes.
- Refreshed by **real user activity only** - no background timer, no keep-alive on an idle tab.
  The same rule the cost section applies to token refresh, for the same reason: manufactured
  activity is indistinguishable from real activity to anything downstream.
- Zeroised on logout, on session end, on idle expiry, and - since collaborative editing replaced
  exclusive leases on 22 September - when the last client disconnects from a document. The
  concurrency ceiling below now bounds **open documents** rather than open sessions.
- Memory only. Never written to disk, never logged, never carried into a crash dump. The key
  service and the backend run with swap disabled and core dumps off, or the key reaches disk by
  accident rather than by design - the same class of mistake as putting key material in a
  Kubernetes Secret.

**What replaces per-operation logging as the control.** The key service already rate-limits. It
gains a **concurrency ceiling**: a cap on how many distinct FAU keys one backend instance may hold
at once, and an alert when the number of newly acquired keys per hour crosses a threshold. That is
the actual mass-decryption signal, and it is bounded rather than merely observed - a compromised
backend hits a wall instead of being written about in a log nobody reads.

**Encryption is server-side, and this is where that becomes visible.** The editor is a React page,
but it never holds the FAU key: it sends document JSON to our backend over TLS, and the backend
encrypts before storage. This is not end-to-end encryption and was never claimed to be. It follows
directly from the trust model - we hold the keys, so encryption defends against a leak and not
against us - and it is why the phrase "we cannot see your data" stays forbidden.

**Notifying an idle viewer costs no decryption at all.** Editing is exclusive by #3420, so a second
person is a viewer or is waiting for the lease. Both need to know the document changed, and neither
needs plaintext to be told: a change notification carries the document identifier and a revision
number, and decision 6 already keeps identifiers, revisions and timestamps in plaintext. The
notification is metadata. Only the viewer's subsequent fetch touches ciphertext, and by then they
are acting, which re-establishes the session key handle under the activity rule above.

Concretely that is a small server-sent-events stream per FAU emitting revision bumps and lease
changes - which htmx consumes natively, so it needs no React outside the editor page. Simultaneous
co-editing remains deferred; this is notification, not collaboration.

### 6. What is encrypted, and what is not

**Encrypted under the FAU data key:** document bodies, change sets, uploaded object bytes,
document titles, and original filenames.

**Sealed to the FAU, decided 24 September 2026:** free text sent *into* an FAU by someone who is
not a member. The first case is the message on an access request (flow spec §5.2). No member session
exists when the message arrives, so it cannot be encrypted under the session-held key. Instead:

- **Each FAU has a sealing key pair.** Both halves live in the key service, never in the application
  database. The private key is wrapped by the FAU's KEK like every other FAU key. The public key is
  not secret, and the key service hands it out freely.
- **On submission,** the backend seals the text to the FAU's public key (a libsodium sealed box, or
  HPKE). It can encrypt but never read back what it sealed.
- **On reading,** the only place is the approval screen, inside an admin's session. The backend asks
  the key service to unwrap the FAU's private key for that session. The call is logged and
  rate-limited like a document-key unwrap, and the key is held under the rules in decision 5a.
- **Never in email.** Decrypting for mail would need the key outside any member session, and would
  hand the plaintext to the mail provider. Admin notifications carry ids only.
- **Deletion:** crypto-shredding destroys the KEK, and every sealed message becomes unreadable,
  including in backups.

The sender's email address stays plaintext; it is an account-level identifier, as above. Until the
key service exists, nothing accepts the message field, so no plaintext is ever stored.

Titles and filenames are inside the boundary deliberately. "Klage på lærer Hansen" or
`bekymringsmelding-elev.pdf` discloses as much as the file it names, and a title sitting in
plaintext sits in every backup.

**Plaintext, because the product cannot work otherwise:** member emails and names, roles and their
date ranges, FAU and school names, document identifiers, sizes, hashes and content types,
timestamps, and audit event types.

Consequences accepted with the boundary: there is no cross-FAU search, by construction, including
for us.

**Search in the MVP is filename search, decided 22 September 2026**, designed for extension to full
text. Nothing moves out of the encryption boundary to make it work: filenames are decrypted inside
an authorised session and filtered there. The FAU key is already unwrapped for that session, so
this exposes nothing the session did not already have.

Decrypt-on-demand beats building an encrypted filename index at this scale, and the arithmetic is
not close. An FAU producing 500 documents holds perhaps 30 KB of filename ciphertext; decrypting
all of it is sub-millisecond, against an index that must be kept consistent with every rename and
deletion. An index is also not an escape from the problem - a plaintext index of filenames
discloses exactly what encrypting filenames was meant to prevent, so it would have to be encrypted
too, and a searchable-encryption scheme leaks frequency and access patterns in exchange. Keep the
filter behind an interface so a real index can replace it if a tenant ever grows enough to need
one. docs/storage-security-proposal.md is otherwise unchanged -
server-generated object keys, short-lived signed uploads, backend-proxied downloads - with
encryption applied before bytes reach object storage, so Hetzner stores ciphertext.

### 6a. How long member data is kept

Decided 22 September 2026, replacing the flat 24-month figure the first draft proposed.

Retention follows **membership, not the account**. An email address is retained for as long as the
person holds any active membership in any FAU. The membership record itself follows the role's own
term - a two-year seat is kept two years, a one-year seat twelve months - and roles updated
mid-term extend to the new role's expiry, which the date-ranged model from #3412 already expresses
without anything new.

When a person's last membership ends, the account lapses after **three months**. Three rather than
six because it still survives the summer holiday, which is the gap the rule exists to cross: a
parent whose seat ends in June and who is re-elected in August is recognised rather than starting
over. After that they sign up again from scratch.

The distinction that makes this correct: accounts are **global across FAU-er**, so a person serving
on a second school's FAU must not have their account swept because their first membership expired.
The lapse test is "no active membership anywhere", not "no activity in this FAU".

**Member-elected retention is designed for and not yet built.** Erik raised the real case: an FAU
taking on extra helpers for a few weeks' project, some of whom expect to return next year and would
rather we kept their data than re-registered. Letting the person choose is better data protection
than a fixed rule imposed on them - it is consent with agency rather than policy by default - but
it turns retention from a sweep into a per-person promise we must store, honour, expire and allow
to be withdrawn.

So the first migration carries a per-account retention field with the default value, and the
MVP ships the fixed rule. Electing a longer period then becomes a screen and a background job
rather than a schema change on live data, which is the difference between cheap and expensive.
Two properties to build in when it ships: an elected retention has to expire and be re-confirmed,
or "keep my data a year" quietly becomes indefinite; and withdrawal has to take effect promptly,
because consent that cannot be withdrawn is not consent.

### 7. Deletion, and the backup that has to survive ransomware

Rewritten 22 September 2026. The original claim was that destroying an FAU's wrapped data key
made its content unrecoverable "including in every backup already written". Codex's review found
that false, and it was: the wrapped key sat on the tenant row, so it sat in every database backup
too, and the master key that unwraps it was still alive. Restore last night's dump and the content
comes back. Erik accepted softening the wording and implementing the design that makes the claim
true.

**The claim, stated so it is provable.** Destroying an FAU's key-encryption key makes its content
unrecoverable, in the live system and in every database backup, **once the queued destruction has
completed**. Until then it is recoverable, and the recovery is itself audited. That is a
cryptographic fact with a bounded window rather than an unbounded promise, and it is what
"fullstendig sletting ved avsluttet kundeforhold" in prosjektgrunnlag.md section 13 should be read
to mean.

Content ciphertext needs no special handling and can sit in versioned or immutable storage
indefinitely, which is the payoff: the rest of the backup design gets to follow ordinary hygiene,
because only the key store carries the deletion requirement.

**Why there is a KEK backup at all.** Erik's requirement, 22 September: if ransomware reaches the
key service, a cluster with no key backup means every FAU's data is gone permanently, which is a
worse failure than the one crypto-shredding defends against. So the KEK store is replicated. The
replica is what makes deletion hard, and the queue is what makes it honest.

**The shape.**

- The replica holds **per-FAU KEK records**, individually addressable, in a store that supports
  real deletes. It is not a generational snapshot history: deleting from today's snapshot does
  nothing about last week's, so there are no weekly snapshots of key material. One live replica
  that deletion propagates into, not a backup timeline.
- The replica's records are encrypted under a **separate backup root key that is not on the
  cluster** and not in the replica. Possession of the replica alone yields nothing. This matters
  because the replica is otherwise a second copy of the crown jewels, and the design should not
  double the attack surface to buy durability. Where that key lives is open item 3 below; an
  offline password manager is the obvious answer and connects to #3481.
- The replica uses **different credentials from the key service**, so one compromise does not
  reach both.

**Deletion is queued, not immediate.**

- Destroying an FAU's key removes the KEK from the live key service at once, and **enqueues** its
  removal from the replica after a delay.
- During the delay the deletion can be cancelled, which is what saves us from an accidental
  destruction and from a destruction we did not authorise.
- The delay is the safety window against ransomware: an attacker who destroys keys has not
  destroyed the replica, and we have the window to notice and cancel.
- **The window is seven days**, revised 22 September 2026 on Erik's instruction that it be at
  least 48 hours and probably 72 or more. His reasoning is the right one and stronger than the
  overnight argument it replaced: a destruction scheduled late on a Friday evening must not
  complete before anyone is back at work. Seven days is proposed over 72 hours because 72 hours
  does not survive a Norwegian Easter - Skjærtorsdag through 2. påskedag is five days, and a
  destruction enqueued on the Wednesday evening would complete untouched. Seven days covers every
  weekend and every holiday cluster in the school year with one number and no calendar logic.
  Lengthening it costs nothing except how long deletion takes to become final, and buys more time
  to catch an attack.
- **A queued deletion is therefore a high-signal alert, not a log line.** An attacker who enqueues
  deletions for every FAU and waits out the window defeats the whole design in silence, so the
  queue must page a human - critical routing on #3442, the path that reaches Signal. Bulk
  enqueueing must be rate-limited the same way bulk unwrapping is, and for the same reason.
- Cancellation is itself audited and alerted, because "the deletion you queued was cancelled" is
  exactly what a successful attacker wants to happen quietly.
- The queue must be executed by the replica side rather than driven from the live service, so that
  compromising the key service does not also grant the ability to cancel.

Key destruction stays two-step, audited, and irreversible once the window closes. A mistaken key
destruction is indistinguishable from data loss, and after the window there is no undo.

**How this is proven**, extending the rehearsal in #3425 rather than inventing a separate exercise:
take a database backup before destruction; destroy the key; restore the backup into an isolated
namespace pointed at the live key service. Before the window closes, the content must still be
recoverable through the documented recovery path - that is the ransomware property. After it
closes, decryption must fail. And the negative control, which is the half that gets skipped: the
same restore against an FAU whose key was never destroyed must succeed, or "decryption failed"
proves nothing but a broken test.

### 7a. The delete request: freeze first, destroy later

Added 22 September 2026 from Erik's instruction. Key destruction is the last step of a deletion,
not the first, and the steps before it are governance rather than cryptography. Keeping them
separate matters because they defend against different things: the verification wait below stops
a deletion nobody legitimately asked for, while decision 7's seven-day replica window is the last
line against a destruction that bypassed this flow entirely - ransomware, or a compromised key
service. Neither replaces the other.

**The request freezes the FAU immediately.** From the moment deletion is requested, and before any
verification completes:

- **Billing stops.** Nobody pays for a wind-down they asked to end.
- **Invitations stop.** No new member can be seated into an FAU on its way out.
- **Write access stops.** The FAU becomes read-only.
- **Read access continues**, deliberately. The freeze is exactly when members should be exporting
  what they need, and a deletion flow that removes access before it removes data is hostile. The
  product should say so at this point rather than leaving people to work it out.

The freeze makes a long wait cheap. The FAU is costing us nothing and costing them nothing
irreversible, so there is no pressure to hurry the confirmation.

**Who must confirm depends on what the FAU has been**, not on who is asking today:

- **An FAU that has only ever had one member** may be deleted by that member without external
  confirmation. There is no shared history to destroy and nobody else to harm.
- **Every other FAU** requires confirmation, because the last remaining member of a council that
  once had ten is not entitled to destroy nine other people's record of it. The test is on the
  membership *history*, not the current count - which is answerable precisely, because the
  approved tenant model in #3412 keeps date-ranged roles rather than a current-members list.
- **Where the recovery contact is a school representative**, confirmation is requested from that
  representative and we **wait at least seven days for a response**. Silence is not consent: an
  unanswered request stays pending, frozen, rather than proceeding by default.

**School holidays extend the wait rather than defeating it.** An FAU is dormant through the
Norwegian summer holiday, and a confirmation request sent in mid-July may reach nobody until
mid-August. A seven-day clock running through it would turn verification into a formality. The
rule proposed: if the wait would expire inside a defined school-holiday period, the clock restarts
at the end of that period. The freeze means this costs the FAU nothing.

**Two different things are called deletion, and conflating them would be a real error.** Closing an
FAU account is a tenant-lifecycle event, and a wind-down measured in weeks is normal and
defensible. An individual exercising erasure under GDPR Article 17 is a different request with a
statutory response time of one month, and it concerns that person's personal data - their address,
their membership record - not the FAU's shared documents, which they do not own and cannot
unilaterally destroy. The flow above is the first. The second needs its own specification on
#3426, and must not inherit these waits.

### 7b. Purging one document, and what crypto-shredding does not reach

Added 22 September 2026, when collaborative editing made per-document history a thing that has to
be removable.

**Purging a document is destroying its key.** Erik's requirement is to remove a document from
history entirely, and his first instinct was a cleaner that deletes it from backups. A cleaner that
must find and delete rows inside every backup is unreliable by construction - it is the same
mistake the original section 7 made. Destroying the document key instead makes that document's step
batches and checkpoints unrecoverable everywhere, including in backups already written, with
nothing to find and no cleaner to run.

It uses the same machinery as FAU deletion: the key is removed from the live key service at once
and its removal from the replica is queued for the same seven-day window, cancellable, alerted.

**In-document redaction is a different operation.** Removing one sentence from a document's history
while the document survives cannot be done by destroying a key, because the surrounding content
must remain readable. It is done by rewriting the step log from a checkpoint, which is why step-log
compaction in the document architecture is a privacy mechanism and not only a performance one.
Two requirements, two mechanisms; neither substitutes for the other.

**What crypto-shredding does not reach: published output.** A published document is a deliberately
released derivative and sits outside the encryption boundary by design, so it is not encrypted
under the document key and destroying that key leaves it untouched. Publication therefore needs its
own deletion path, and the store holding published artifacts must permit real deletes rather than
carrying the permanent-deletion-denied policy used on `fau-tfstate` - the same constraint as the
key replica, for the same reason. Stated here because "we destroyed the key, so it is gone" is
exactly the sentence someone will say about a document that is still sitting on the public web.

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

**Choosing neither is not permitted**, decided by Erik 22 September 2026. Every FAU has a recovery
contact, because the alternative is an FAU with no route back from losing its last member, and a
public school is legally required to have an active FAU. **We hold the seat until a nominated
school representative has been confirmed**, so the transition has no gap in which nobody holds it.
A nomination that is never confirmed leaves us in the seat indefinitely, which is the safe failure.

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
- **#3425 backup and isolated restore**: gains the key-destruction rehearsal in decision 7, and
  gains the rule that the key replica is a live replica rather than a generational backup. Also
  gains a bucket-policy check: the replica's store must permit real deletes, so it must not carry
  the permanent-deletion-denied policy used on `fau-tfstate`.
- **#3442 alerting**: gains queued key destruction, bulk enqueueing and cancellation as critical
  alerts. These are the events where a silent failure is indistinguishable from an attack.
- **#3481 Proton Pass export**: gains a second trigger. The backup root key from decision 7 must
  live off the cluster, and an offline password manager is where that points.
- **#3409 supplier mapping**: gains Hanko GmbH as a customer-facing processor in Germany, which
  satisfies the European rule rather than excepting it. Needs the Hanko Cloud DPA signed, and
  records AWS, Hetzner and adesso as a service as its infrastructure subprocessors - AWS being
  US-owned, though the data stays in the EU and no Article 44 transfer arises. No exception entry
  is required for identity any more.
- **#3410 transactional email**: recovery notifications are transactional mail to people who may
  no longer be members, which the provider evaluation should account for. Authentication mail is
  not part of that scope - Hanko sends the passcode itself, from EU infrastructure.
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
| Privileged action attempted by an account without TOTP enrolled | Denied; user sent to MFA setup |
| Privileged action attempted in a session older than the freshness threshold | Denied; re-login forced |
| Authorization decision derived from a provider-minted claim | Must not exist; caught by review and by test |
| Provider `sub` used as a key in any application table other than the mapping | Must not exist; caught by schema review |
| One internal user given a second (issuer, subject) mapping | Both identities resolve to the same internal user; memberships, roles and content unchanged |
| Two issuers accepted simultaneously | Members authenticate through either; no duplicate accounts created |
| Authentication attempted without PKCE | Rejected |
| FAU key destroyed, window elapsed, restore of an older database backup attempted | Content unrecoverable |
| FAU key destroyed, restore attempted *before* the window closes | Content recoverable through the documented path, and the recovery audited |
| Control: restore of the same backup for an FAU whose key was never destroyed | Content recoverable — without this the test above proves nothing |
| Key service datastore destroyed entirely, replica and backup root key available | Every non-deleted FAU restored |
| Replica obtained without the backup root key | Yields nothing usable |
| Bulk destruction enqueued for many FAU-er | Rate limit trips; critical alert pages a human before the window closes |
| Queued deletion cancelled | Audited and alerted; key remains usable |
| Delete requested | Billing stops, invitations stop, writes refused, reads still succeed |
| Last remaining member of an FAU that once had several requests deletion | Confirmation required; not self-service |
| Sole-ever member requests deletion | Proceeds without external confirmation |
| School representative does not answer a confirmation request | FAU stays frozen and pending; deletion does not proceed by default |
| Confirmation wait would expire inside the summer holiday | Clock restarts after the holiday; FAU stays frozen |
| Key material found in a Kubernetes Secret or an etcd snapshot | Must not exist; caught by review |
| Document key found on a document row or anywhere in the application database | Must not exist; caught by schema review |
| One document's key destroyed | That document unrecoverable; every other document in the FAU unaffected |
| Document key destroyed after the document was published | Private source unrecoverable; published artifact still present until its own deletion runs |
| Key handle still resident after logout, idle expiry or lease release | Must not exist; zeroised |
| Idle tab or background timer refreshing a key handle | Must not exist; only real activity refreshes |
| Backend instance asked to hold more FAU keys than the ceiling | Refused; alert raised |
| Change notification inspected on the wire | Carries identifier and revision only; no plaintext |
| Account with an expired membership in one FAU and an active one in another | Retained; not swept |
| Account whose last membership ended more than three months ago | Lapsed; re-registration required |

## Open items

Everything asked on #3484 has now been answered. What remains is one deferred build and two
standing risks worth keeping visible rather than closing.

1. **Member-elected retention.** Designed for in decision 6a, not built in the MVP. The schema
   carries the field from the first migration; the screen, the expiry-and-reconfirmation and the
   withdrawal path come later. Raise it again when there is a real FAU asking.

### Standing risks, recorded rather than resolved

- **A single operator.** Cancelling a queued key destruction takes one operator, decided
  22 September because there is currently only one person who could act. That is the right call
  today and a concentration worth naming: the same person holds the Proton Pass credential from
  open item 3, sits in the recovery seat for every FAU that has not confirmed a school
  representative, and is the only one who can stop a destruction inside its window. No single
  control fixes this; a second trusted operator does, and the design should be revisited the day
  one exists rather than the day one is needed.
- **ISO 27001 at Hanko is in progress, not achieved**, per Hanko's own statement. Nothing depends
  on it, and nothing should be claimed on its behalf until it is certified.

### Closed

- **Zitadel free-tier metric.** Moot twice over; the provider changed twice.
- **Finding a European provider for identity.** Answered by the provider itself: Hanko is German.
- **Administrative two-factor.** Answered on #3414 and written into decision 4.
- **What the provider is allowed to hold.** Settled structurally: Hanko has no organisation model,
  so it holds an email address and authentication material and nothing else.
- **Who sends authentication email.** Hanko sends the passcode from EU infrastructure.
- **Magic link or emailed code.** Emailed passcode, decided 22 September - Hanko's native
  mechanism, and the one that survives mail scanners.
- **Passkeys in the MVP.** No. Passcode only, because the users are parents on whatever device
  they own and a passkey on a replaced phone is a support call. Revisit after launch.
- **Section 7's deletion claim.** Softened to something provable, with the design that makes it
  true in decisions 5, 7 and 7a.
- **The two deletion timers.** Replica window seven days; school-representative confirmation wait
  seven days. **Only the summer holiday restarts the clock** - decided 22 September, on the
  grounds that seven days plus monitoring every other day already covers Christmas and Easter.
  Cancelling a queued destruction takes **one operator**.
- **Custody of the backup root key.** Proton Pass, which makes this a second trigger for #3481
  alongside the SOPS age key.
- **Search in the MVP.** Filenames only, decrypted in-session, designed for extension to full
  text. Nothing leaves the encryption boundary - see decision 6.
- **Retention of member data.** Per-membership, lapsing three months after the last membership
  ends - see decision 6a.
- **Key service failure modes.** Answered by decision 5a: a session-scoped key handle in memory,
  refreshed by real activity, with a concurrency ceiling as the control that replaces
  per-operation logging.
- **Recovery contact for FAU-er that choose neither.** Not permitted. Every FAU chooses us or a
  school representative, and we hold the seat until a nominated representative is confirmed.
