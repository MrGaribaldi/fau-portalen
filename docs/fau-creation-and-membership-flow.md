# FAU creation and membership flow: design for #3413

Status: drafted 23 September 2026 from a design session with Erik. Every section was agreed in that
session; the document is for his review as a whole. Written in English per CLAUDE.md; user-facing
strings are Bokmål source strings for the localisation mechanism (#3439), shown here in quotes.

## 1. Scope

This is the product flow that creates an FAU and brings people into it: signup, verification,
activation, invitations, roles and access, the yearly admin turnover, and what happens when an FAU
has no administrator left. It fixes the rules. Implementation is split between #3417 and #3418
(section 8), and the data model those cards add is outlined in section 9.

**Out of scope, and not to be assumed:**

- Member-initiated invitations *other than* the replacement proposal in 5.3, substitute (vikar)
  invitations, and anything KFAU-specific.
- Bulk school-year changes: moving cohorts up a grade, merging groups, shifting role dates in bulk.
  Initial units and cohorts belong to #3418's school setup and are not specified here.
- Email change. #3437 owns it and its MVP priority is open. Nothing in this flow may assume an
  account can change address, and nothing may make it harder: the account ID is the identity, the
  email is a login address.
- Passkeys. Passcode only in the MVP (decision 12).
- An individual's Article 17 erasure (#3426) and closing an FAU (ADR-003 decision 7a) are separate
  flows with separate clocks.

**Sources.** Where this document restates an earlier decision it cites it; where it decides something
new the decision is listed in section 11.

- prosjektgrunnlag.md
- docs/planning-decisions.md; later entries override earlier ones
- docs/tenant-role-history-design.md (#3412, approved 7 September)
- docs/identity-and-encryption.md (ADR-003)
- docs/url-scheme.md (ADR-002)
- #3414, admin two-factor
- backend/migrations/0002_identity_and_tenancy.sql

## 2. Principles that bind every section

1. **Access needs two things:** a verified email, and at least one role that is valid today. "Today"
   means the server's calendar date in Europe/Oslo. Rights are the union of all roles valid today,
   and they are re-checked on every request, never cached from login (#3412; ADR-003 decision 4).
2. **Role periods are half-open date ranges with a mandatory end.** A future-dated role grants nothing
   before it starts (#3412; enforced by 0002).
3. **The capability class decides privilege, not the role name.** A role called "Leder" with class
   `member` grants member rights only.
4. **Only admins grant access**, with three bounded exceptions that are named where they occur:
   - the leader invitation issued at activation (3.4);
   - a handover invitation from an outgoing admin (6.3);
   - the recovery contact in the no-admin case (6.4).
5. **Privileged admin actions go through #3414's gate.** Creating an invitation, approving a
   request, granting or revoking a role, and any recovery-contact action all require:
   - an enrolled second factor (TOTP in the MVP);
   - a fresh session. The freshness threshold is 30–60 minutes; the exact number is open on #3414.
6. **Every state change is one transaction with its audit entry and its outbox message.** A mail
   failure delays a notification but never loses it, and never repeats the state change.
7. **No response ever reveals who is in an FAU** to someone outside it.

## 3. Signup and activation

### 3.1 The form

1. **School.** The registrant picks it from the national school register (#3441): municipality first,
   then school.
   - If the school is not listed, "Mangler skolen din?" creates an unverified school on the spot, as
     docs/url-scheme.md specifies. It is reachable only at `/s/<uuid>`, marked `noindex`, and queued
     for Erik's review, with a turnaround of about a week (ADR-002, 10 September).
   - An unverified school works fully.
2. **FAU name.** Pre-filled from the school name, and editable.
3. **Registrant's email.**
4. **Leader's email.** It may be the registrant's own.
5. **Admin end date.** The form asks "Til hvilken dato er du valgt?".
   - The default is 1 October of the next FAU year: the first 1 October at least three months after
     today. FAU-er are usually elected at the autumn parents' meeting.
   - Allowed range: from one month to 24 months after today.
   - An end date is mandatory. #3412 forbids an indefinite bootstrap admin.
6. **Consent.** A line linking the terms and privacy notice (#3427).

A pre-filled outreach link (`/bli-med/<uuid>`) may pre-select the school. That is a correctable
suggestion: it grants nothing, carries no personal data, and cannot claim an existing FAU (#3441).

### 3.2 A school that already has an FAU

The database enforces one FAU per school, and a pending FAU counts (section 9).

- **The existing FAU is active.** The registrant sees "Skolen har allerede et FAU på portalen. Kontakt
  en administrator for å få tilgang." They may then ask the portal to pass the request on, which
  creates an *access request* (5.2):
  - The requester first confirms their own address with a Hanko passcode, so nobody can make the
    portal email an FAU's admins from an address they don't control.
  - The requester never learns who the admins are.
- **The existing FAU is pending.** The registrant sees "Denne skolen er i ferd med å bli registrert."
  No relay is offered, because a pending FAU has no admins yet.
- **Both cases:** a copy goes to `fau@ewb-solutions.as`, so Erik sees every collision. This is the
  manual review he chose; the relay lets an active FAU resolve most collisions itself.

### 3.3 Pending

Submitting the form creates the FAU in `pending`, with the school and the form's values. The
registrant then receives a Hanko email passcode, and has no data rights until they enter it (#3412).

A pending FAU that is not verified within **7 days** expires. The row is deleted rather than
closed, since it never became an FAU, and the school is free again. The collision copy from 3.2
lets Erik see if expiries are being used to squat. One email address may hold at most three
pending FAU-er at a time.

### 3.4 Activation

Entering the passcode activates the FAU. One transaction does all of this:

1. The FAU becomes `active`.
2. The registrant gets a membership and a role with class `admin`, named "Administrator", from today
   to the chosen end date.
3. The leader invitation is issued, unless the leader address is the registrant's own:
   - It carries an admin role with the same end date. The leader may shorten or lengthen it on
     acceptance, within the same 1–24 month range.
   - It is valid for 14 days, like any invitation.
   - Its mode is `activation`, and it needs no TOTP, because it completes the signup form rather
     than being a new admin action (decision 11).
4. The recovery-contact seat is set to FAU/EWB (6.5).
5. An audit entry and an outbox message to Erik are written.

Verifying the registrant never verifies the leader address.

### 3.5 The leader

The leader opens the invitation, logs in with a passcode, adjusts the end date if they want to, and
clicks "Bli med". They then hold their own admin role. If the leader already has an account from
another FAU, it is reused.

### 3.6 One admin is allowed

An FAU works fully with a single admin; a volunteer body that stalls on paperwork simply leaves.
While an FAU has exactly one admin, admins see a persistent banner, "Legg til en administrator til",
with a shortcut to invite one or re-send the leader invitation.

**Assumption.** Nothing verifies that the named leader really is the FAU's elected leader. Unlike the
school representative (ADR-003 decision 9), the leader is taken on trust in the MVP. Erik accepted
this on 23 September.

## 4. Login and FAU switching

- **Login** is a Hanko six-digit email passcode (ADR-003 decision 4a).
- **Sessions** last 30 days. That is safe only because access is re-checked on every request.
- **TOTP enrolment** is offered to every admin on first login. It is enforced the first time the
  admin attempts a gated action (2.5), and then they are sent to set it up.
- **Several FAU-er:** an account that belongs to more than one chooses its FAU at login and switches
  from a menu. Switching carries no rights across.
- **No valid role today:** an account in that state sees "Du har ikke tilgang til dette FAU-et nå"
  and nothing of the FAU. If a role is scheduled to start later, the page gives its start date.

## 5. Invitations and requests

### 5.1 Invitations

- **Who creates them.** An admin, through the gate in 2.5, or one of the exceptions named in 2.4.
- **What an invitation carries:**
  - the recipient's email;
  - one or more roles, each with a start date and a mandatory end date;
  - its mode: `normal`, `activation`, `handover` or `recovery`;
  - the issuer.
- **Lifetime:** valid for **14 days**. The token is single-use and stored only as a hash.
- **Changes:** re-sending issues a new token and revokes the old one. The issuer or any admin can
  withdraw a pending invitation.
- **Acceptance.** The recipient opens the link and sees the FAU name and the roles offered. They log
  in with a passcode and click "Bli med". Opening the link accepts nothing, so mail scanners that
  pre-fetch links are harmless (ADR-003 decision 4a). Acceptance is one transaction that checks:
  - the token is valid, unused and unexpired;
  - the recipient's email is verified and matches;
  - the FAU is active and not frozen;
  - the issuer still has authority for these roles: an admin role valid today, a handover grant
    valid today, or the recovery seat in the no-admin state.

  It then creates or reuses the membership, activates the roles and writes the audit entry.
- **Failures.** Each failed check produces a specific message. None of them names another person.

### 5.2 Access requests

Created from 3.2. An access request holds:

- the target FAU;
- the requester's verified email;
- an optional short message: plain text, maximum 500 characters, always escaped.

Handling:

1. The FAU's current admins are emailed "`<address>` ber om tilgang til `<FAU>`", with a button that
   opens a pre-filled invitation.
2. The admin chooses the roles, and approving issues a normal invitation (5.1).
3. The admin may instead decline. The requester is told "Forespørselen ble ikke godkjent" and
   nothing more.
4. A request that nobody handles lapses after 30 days, and the requester is told the same.

### 5.3 Replacement proposals

A member may propose a successor for one of their own roles. This is new in the MVP and supersedes
the 7 September deferral (decision 13).

The proposal holds:

- the successor's email;
- the role, whose name and capability class are copied from the proposer's role and cannot be
  changed;
- dates starting no earlier than today, suggested to start when the proposer's role ends.

It goes to the admins exactly like an access request, and approving it issues a normal invitation.
The admin may adjust the dates, or the role, before approving.

- A member can propose only for a role they hold today.
- Proposing does not end the proposer's own role.
- A proposal for an `admin`-class role still needs an admin to approve it. Admins in the handover
  window use 6.3 instead.

### 5.4 One request model

Access requests and replacement proposals are one model. They share a table with a `kind` column,
one approval screen, and the same statuses: pending, approved, declined, withdrawn or lapsed.

Limits, as starting values for #3417 and #3418 to tune:

- one open request per address per FAU;
- five requests per FAU per day;
- a replacement proposal counts against its proposer.

## 6. The admin lifecycle

### 6.1 Warnings

- **Each admin** is emailed 30 days and 7 days before their admin role ends.
- **All members** are also emailed when the FAU's last admin role is 30 days from ending with no
  other admin role valid on the day after it ends. Any member could be the successor, or propose
  one (5.3).

### 6.2 The handover window

When an admin role reaches its end date naturally, it produces a handover grant. The grant runs six
calendar months from the exclusive end date, truncated to the last valid day of the month, and the
boundary is computed and stored explicitly (#3412). For example, a role ending 2027-10-01 gives a
handover window of `[2027-10-01, 2028-04-01)`.

A revoked admin role produces no grant, and revoking a role later also ends any grant it already
produced (#3412).

### 6.3 What handover allows

The window lets an outgoing admin do exactly one thing: bring in their replacement.

**Allowed:**
- create, re-send and withdraw their own replacement invitations. These have mode `handover` and
  are linked to the grant. The outgoing admin chooses the new person's roles, including admin.

**Not allowed:**
- extending their own role or invitation;
- seeing the member list;
- revoking anyone;
- approving other requests;
- editing the organisation.

The 2.5 gate still applies. The handover grant gives no document access: if the person also holds
a valid member role, they keep document access through that role only. When the recipient accepts,
they become an ordinary admin.

### 6.4 No admin left

An FAU has no admin when no `admin`-class role is valid today and no handover grant is valid today.
When that happens:

1. The FAU is flagged as without administration.
2. Current members and everyone who held a role in the past 24 months are notified (ADR-003
   decision 10).
3. Members keep whatever access their own roles give.
4. The recovery contact's single power, initiating the addition of a member (ADR-003 decision 8),
   may in this state grant that person an `admin`-class role. This clarifies decision 8 (decision
   10). Everything ADR-003 decision 10 requires applies:
   - step-up re-authentication;
   - a permanent audit entry;
   - an email to every current member, both when the invitation is created and when it is accepted;
   - a 14-day login banner;
   - when no members remain, notices go to recent role-holders and to the second party.
5. The flag clears when any `admin`-class role becomes valid.

### 6.5 The recovery contact

- **At activation** the seat belongs to FAU/EWB.
- **Nomination.** An admin may nominate a school representative. They are verified as ADR-003
  decision 9 describes: a domain email at the school's or municipality's official address, plus a
  manual title check recorded by Erik.
- **EWB keeps the seat** until the nominee is confirmed. An unconfirmed nomination leaves EWB in
  place.
- **What the recovery contact cannot do:** see documents, the audit log or the member list, or give
  itself a role (ADR-003 decision 8).
- **Screens.** The nomination screen belongs to #3418. This document fixes the rules only.

## 7. Leaving and removal

- **Revocation by an admin.** An admin may revoke another person's role or whole membership. It takes
  effect at once, is audited, and creates no handover grant.
- **Leaving.** A member may leave an FAU, which revokes their own membership.
- **Last admin safeguard.** No action may leave the FAU with no admin without an explicit
  confirmation that names the consequence. That covers an admin revoking the only other admin,
  revoking themselves, or leaving. The confirmation reads "FAU-et får da ingen administrator. Bare
  gjenopprettingskontakten kan gi tilgang etterpå."
- **Account retention.** Retention follows ADR-003 decision 6a. An account lapses 3 months after its
  last membership anywhere ends, and a membership ending in one FAU never touches the account's
  standing in another.

## 8. Implementation split

**#3417: signup, verification, activation and login**
- the register picker, plus the "Mangler skolen din?" path;
- the duplicate check (3.2), including the passcode-verified access request submission;
- the pending state and its expiry;
- the activation transaction, including the leader invitation;
- Hanko passcode login, TOTP enrolment and the 2.5 gate;
- the per-request access check;
- the FAU switcher.

**#3418: invitations, requests, roles and the lifecycle**
- invitations, with acceptance, re-send and withdrawal;
- the request model: access and replacement, with admin approval;
- role assignment and revocation;
- leaving, and the last-admin safeguard;
- warnings;
- handover grants;
- the no-admin flag and the recovery-contact admin grant;
- school-representative nomination;
- initial units and cohorts, already in its scope.

#3422 builds the screens for both cards. Every user-facing string goes into the Bokmål catalogue as
a source string (#3439).

## 9. Data model for migration 0003 (shape, not DDL)

#3417 and #3418 write the migration. Every table follows the rules 0002 established, and the schema
review tests already guard them:

- composite `(tenant_id, id)` keys for tenant data;
- explicit grants to the runtime role;
- no key material;
- half-open date ranges with mandatory ends.

- **`municipalities`, `schools`.** Global register data, carrying the stable slugs, provenance and
  verification state from #3441 and docs/url-scheme.md, and the slug history that makes 301 redirects
  work.
  - `tenants.school_id` gets its foreign key.
  - A unique index on `tenants (school_id)` covering `pending` and `active` enforces one FAU per
    school. A closed FAU does not block a new one.
- **`invitations`, `invitation_roles`.**
  - Each invitation has a token hash, its mode, its issuer, an optional link to a handover grant or a
    request, `expires_at`, `accepted_at` and `revoked_at`.
  - `invitation_roles` holds the offered roles and their dates.
- **`access_requests`.** Kind, requester email, an optional requesting membership (for replacements)
  and the role it replaces, status, decider, and timestamps.
- **`handover_grants`.** As in #3412: a unique source assignment, the explicit boundary, and
  `revoked_at`.
- **Recovery contact.** One row per tenant naming the seat holder (EWB or a school representative),
  plus the nomination and its verification record.
- **`audit_events`, `outbox`.** Brought forward from #3421 and #3410 as minimal tables, because
  activation needs both inside one transaction (decision 14).
  - `audit_events` is append-only: the runtime role may insert and select, never update or delete.
  - `outbox` holds pending notifications for a sender to deliver.
  - #3421 later builds the visible audit log and its protections on the same table.

## 10. Testing, the backbone for #3417 and #3418

Each rule above gets a test that fails if the rule breaks. At minimum:

**Signup and activation**
- A second signup for the same school fails, whether the first FAU is pending or active. The message
  names nobody.
- A pending FAU expires after 7 days, and the school is then free.
- Activation is atomic: FAU, role, leader invitation, recovery seat, audit and outbox are written
  together or not at all. A mail failure does not undo activation.

**Invitations and requests**
- Opening an invitation link without logging in and clicking accepts nothing.
- Acceptance fails in each of these cases:
  - the token is expired, used or revoked;
  - the email does not match;
  - the FAU is frozen or not active;
  - the issuer no longer has authority.
- An access request cannot be submitted without passcode verification. No response to the requester
  names an admin.

**Access**
- A role valid tomorrow grants nothing today.
- Losing the last valid role removes access on the next request, not at the next login.

**Handover and the no-admin state**
- A handover grant runs exactly six months from the exclusive end date, including month-end
  truncation.
- Revoking an admin role produces no grant, and ends one it already produced.
- A handover-only admin cannot list members, revoke roles or extend their own role.
- In the no-admin state the recovery contact can grant admin. Outside it, the recovery contact
  cannot.
- The last-admin safeguard requires confirmation in each of these cases: revoking the only other
  admin, revoking oneself, and leaving.

## 11. Decisions

**Made on 23 September 2026, by Erik, in the design session:**

1. **School selection.** The school is chosen from the register, with the "Mangler skolen din?"
   fallback. #3413 therefore depends on #3441's register import.
2. **One FAU per school.** A duplicate attempt is blocked and copied to Erik for manual review.
   - If the existing FAU is active, the registrant is told to contact its admins, and the portal can
     relay a passcode-verified access request to them without revealing who they are.
   - If it is pending, the registrant is told it is being registered.
3. **The first admin end date** is asked at signup. It defaults to the next 1 October at least three
   months away, within a range of 1–24 months. The leader invitation carries the same date, which
   the leader can adjust.
4. **A single admin is allowed**, with a persistent banner while there is only one.
5. **The no-admin case.** Warnings go out first, then the recovery contact may grant admin (decision
   10).
6. **Invitations** are valid for 14 days. Opening a link never accepts it.
7. **The implementation split** is by card (section 8), in one flow document.
8. **The leader is taken on trust** in the MVP (3.6).
9. **The 0003 bridge.** Minimal `audit_events` and `outbox` tables come forward from #3421 and #3410
   (decision 14).

**Clarifications and changes to earlier decisions, to be recorded in docs/planning-decisions.md:**

10. **ADR-003 decision 8 is clarified.** In the no-admin state only, the recovery contact's single
    power (initiating the addition of a member) may grant an `admin`-class role. Outside that state
    the power remains a plain member addition.
11. **The leader invitation** issued at activation is exempt from #3414's TOTP gate. It completes the
    signup form rather than being a new admin action. Every later invitation is gated.
12. **Passkeys are not in the MVP**; passcode only. This settles the inconsistency between ADR-003's
    "Closed" section, which already said no, and its decision 4a and CLAUDE.md, which still called it
    open.
13. **Replacement proposals are in the MVP** (5.3). They supersede the 7 September "Confirmed
    simplifications" deferral for this one case. Other member-initiated invitations, and substitute
    invitations, remain deferred.
14. **Migration 0003 brings forward minimal `audit_events` and `outbox` tables**, so activation does
    not wait for #3421.

## 12. Open, and carried forward

These do not block the spec:

- **The #3414 gate.** The exact freshness threshold, and the list of endpoints it covers.
- **Email change.** #3437's MVP priority.
- **KFAU.** Whether it needs anything distinct.
- **Unverified-school review.** How the queue is tooled for Erik.
- **Starting values.** The rate-limit and lapse values in 5.2 and 5.4 are starting points to tune.
