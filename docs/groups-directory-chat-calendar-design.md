# Groups, directory, chat, calendar and date poll: design

Status: **accepted 26 September 2026** on #3500. Erik reviewed the written spec and accepted it
with one comment: section 5.2 has no live preview in the MVP, and one may be implemented later
(now listed in section 9). The closed-group ruling in section 3.3 was accepted with the rest. Two
features came out of conversations with potential customers, chat and a calendar. During design
they grew into five pieces that share one substrate. Erik's decisions are listed in section 2.
Rulings the agent made where he gave no answer are marked **Ruling**. User-facing strings are
Bokmål source strings for the localisation mechanism (#3439), shown in quotes.

## 1. Scope and build order

| # | Piece | What it is | Depends on |
|---|---|---|---|
| 1 | Groups, guest, authorization | Arbitrary groups inside an FAU, a third capability class `guest`, one authorization function | #3412's units/cohorts |
| 2 | Member directory | Names, what each person represents, contact emails; select people, then mailto or copy | 1 |
| 3 | Threads and chat | Encrypted message threads on the FAU, groups, events and polls | 1 |
| 4 | Calendar | Events and meetings, a redacted subscription feed, a full `.ics` download | 1, 3 (event thread) |
| 5 | Date poll | Doodle-style scheduling that becomes an event | 3, 4 |

Folders are not a piece of their own. They carry an audience (section 3) and **bind the migration
that creates documents (#3419)**, which does not exist yet: migrations stop at `0004`.

Built in that order. Each piece gets its own implementation plan and Favro card; this document is
the shared spec.

## 2. Decisions

| # | Question | Decision |
|---|---|---|
| D1 | What need is behind "chat to discuss meeting times"? | Both a date poll for scheduling and a general chat |
| D2 | Who talks to whom? | Channels (FAU-wide and per group) plus threads on meetings, events and polls. **No direct messages** |
| D3 | Whose email does the directory show? | Every current member's, as a contact address the member may set; the login email by default |
| D4 | Chat retention | **Expire after 12 months** by destroying keys; decisions belong in minutes and documents |
| D5 | Getting events into members' own calendars | A **redacted subscription feed** (times and a generic label only), plus a full `.ics` download inside the session |
| D6 | A public calendar for parents | Design for it now (a `visibility` column), build it later |
| D7 | Notifications outside the app | **In-app only in the MVP.** An email digest may come later as a paid add-on per FAU; web push is possible after that. FAU-er also reach each other outside the portal |
| D8 | Date poll behaviour | Open votes (yes / if need be / no); the organiser decides; nothing closes automatically |
| D9 | Groups beyond years | Any group, for example a committee, a year or a working group. Years and units are just one way to fill one |
| D10 | A third member type | **Guest**: reads and writes only within their own groups and folders |
| D11 | Can members see other groups? | **Open to members by default**, and an admin can mark a group closed. Admins see everything; guests see only their own groups |
| D12 | Chat formatting | Bold, italic and underline, links, and links to existing documents |

## 3. Groups, audience and authorization

### 3.1 Tables

All tables carry `tenant_id`, and every reference is composite, as in `0002`.

- `groups`: `id`, encrypted `name`, `visibility` (`open` | `closed`), optional `unit_id` /
  `cohort_id` binding, `archived_at`. Names are encrypted like document titles, because a name
  such as "Oppfølging av sak med rektor" is content.
- `group_members`: `group_id`, `membership_id`, `added_by`, `added_at`, `removed_at`. Removal is
  soft and audited, because group history is FAU history.
- **Membership of a bound group is derived, not copied:** it is everyone holding a role on that
  unit or cohort on the current Europe/Oslo date. Turnover then needs no job. A bound group can
  still have manual members added, such as a guest teacher on 7. trinn.
- `roles.capability_class` gains `'guest'`, and `CapabilityClass` gains `Guest`. A check ensures
  that a guest role names at least one group.
- #3412's `organization_unit`, `cohort` and `unit_cohort` land first, in the same migration or
  just before it. The foreign keys that `0002` left open on `roles.unit_id` / `roles.cohort_id`
  close there.

### 3.2 Audience

Every resource that carries an audience has one nullable `group_id`. That covers threads, events,
polls and folders, and a directory entry through its memberships. `null` means FAU-wide: members
and admins, **never guests**. There is one column, not a join table: a resource meant for two
audiences is two resources.

### 3.3 The authorization function

There is a single function, called once per operation, that always reads the database.

| Viewer | FAU-wide | Open group | Closed group |
|---|---|---|---|
| Admin | read + write | read + write | read + write |
| Member | read + write | read; write if in the group | read + write only if in the group |
| Guest | nothing | read + write only if in the group | read + write only if in the group |

- "Write" means posting, voting and editing content.
- **Managing groups is admin-only in the MVP.** That covers creating, closing and archiving a
  group, adding members, and inviting guests. A group lead role is deferred.
- **Ruling:** a closed group's existence and name are hidden from anyone who cannot read it.
  Its name is content, and knowing that "a closed group about X exists" is itself a disclosure.
- The per-FAU SSE stream is filtered through the same function. Otherwise a guest would learn
  that a closed thread just got a message.
- When a role or group membership is revoked, it takes effect on the next request and closes
  live SSE connections. That matches the rule #3490 set for WebSockets.
- **Before `Guest` exists**, every read path in the #3418 membership code is audited for an
  "active membership = read everything" shortcut and routed through this function.

### 3.4 Encryption

Unchanged. Keys stay per FAU and per document, held server-side (ADR-003), so guest restriction is
authorization, not cryptography. Per-document keys would allow per-group keys later if ever needed;
nothing here builds that.

## 4. Member directory

### 4.1 Data

Both fields are per membership, so one person can go by different names in two FAU-er, and both
are encrypted under the FAU key:

- **`display_name`**: required when an invitation is accepted. The member can edit it, and an
  admin can correct it. This is the first place names are stored anywhere in the system; Hanko
  still holds none.
- **`contact_email`**: optional; the login email is shown when it is empty. **No system mail is
  ever sent to it.** Login, invitations and recovery keep using the account email. It is
  therefore not verified: it is the member's own statement, used by other members' mail clients.

What a person represents is derived from their current role assignments (role name plus unit or
cohort) and their group memberships. It is never typed separately. Past roles are not shown,
because the directory shows the present.

### 4.2 Retention

Amended 28 September 2026 by Erik's decision D3 (docs/planning-decisions.md), which replaces the
earlier rule that the name stays after the membership ends.

- **A display name exists only while the membership is active.** When the membership ends, both
  the display name and the contact email are cleared, and the database enforces this on
  revocation. The definition of "ended" is in the #3502 plan: revoked, or no running or upcoming
  role.
- **History shows an ended membership as role and year**, for example "Leder 2025–2026", never
  as a name. The label is computed at read time from the role assignments. Erik's reason: it
  keeps the history GDPR-valid. This is what satisfies prosjektgrunnlag §8's "the names that
  applied at the time" for former members, and it must be written into the privacy notice and
  the DPA (#3426).
- An Article 17 erasure shows "Tidligere medlem", never role and year, because a single-holder
  role with its year would still identify the person.
- Crypto-shredding the FAU removes everything.

### 4.3 Screen

Rust-rendered with htmx, plus a small TypeScript module for selection.

- The list is grouped by group. Each entry shows the name, what the person represents, and their
  email, with guests marked "Gjest".
- Names are sorted with locale-aware collation, never byte order (#3439: no assumption of Latin
  collation).
- There is a checkbox per person, "Velg alle" per group, and a filter by group. A person selected
  through two groups appears once.
- **"Skriv e-post"** builds a `mailto:` link. A To/Bcc toggle defaults to **Bcc** when more than
  10 people are selected.
- **"Kopier adresser"** uses `navigator.clipboard`, falling back to a selectable text box.
- **Length guard:** mail clients (notably on Windows) truncate `mailto:` URLs at about 2,000
  characters. When the URL would pass 1,800, "Skriv e-post" is disabled and a short explanation
  points to copying.
- Addresses are joined with `, ` per RFC 6068. Outlook desktop's handling of commas and semicolons
  is checked during implementation, and the copy format can switch to `; ` if needed.

### 4.4 Audit and guests

- Every mailto or copy action writes an audit entry with the count and the group, **never the
  addresses**. Pulling out a batch of addresses is a disclosure.
- A guest sees only the members of their own groups. Members see guests as part of those groups,
  with their address. Being reachable by the group comes with joining it, and the guest
  invitation text says so.

## 5. Threads and chat

### 5.1 Model

**There is no channel table.** A thread belongs to a subject: the FAU, a group, an event or a
poll. The subject decides its audience. "Hele FAU" and every group get one thread each,
automatically. A topic channel such as "Dugnad" is an open group, so both the concept and the
authorization rule stay single.

`messages` carries `thread_id`, `seq` (gapless per thread, taken under a row lock on the thread),
`author_membership_id`, `created_at`, `edited_at`, and an encrypted `body`.

### 5.2 Text format

This format is shared with event and poll descriptions.

- **Syntax:** `**fet**`, `*kursiv*`, `++understreket++` (underline isn't in standard Markdown; `++`
  is the markdown-it-ins convention), `[tekst](url)` and line breaks. Links accept the `http`,
  `https` and `mailto` schemes only. There are no headings, lists, images or embedded HTML.
- **Document links are stored as `[[doc:<uuid>]]`, never as titles.** The composer's "Lenk til
  dokument" picker uses the MVP filename search, inside the session. When a message is shown, the
  server resolves each link **for that viewer**:
  - the current title, if the viewer may read the document;
  - "Dokument du ikke har tilgang til", with no title, if they may not;
  - "Slettet dokument", if the document has been purged.

  A renamed document keeps working, and an encrypted title is never copied into a chat body.
- **One Rust parser** produces a small AST: bold, italic and underline, which can be combined,
  plus links and text. HTML is built from the AST with escaping, so raw `<` never passes through.
  The **source text** is stored, encrypted, and rendered again on every read, which is what keeps
  the per-viewer resolution of document links correct.
- **On write**, the server rejects document links to another FAU or to ids that do not exist.
- **Composer:** a textarea with B/I/U buttons and Ctrl+B/I/U shortcuts that insert the markers,
  plus the document picker. There is no live preview in the MVP; the sent message shows the
  result.
- Links carry `rel="noopener nofollow"`. There are **no link previews**, since fetching a URL
  server-side invites SSRF and tells the linked site that we exist.

### 5.3 Behaviour

- **No attachments**; a message links to a document instead, so uploads keep one path through
  the ingest scanner (#3447).
- Messages are capped at 4,000 characters, and posting is rate-limited per member.
- There are no mentions, reactions or read receipts in the MVP. Mentions become useful only with
  notifications, meaning the paid digest.
- Members can edit and delete their own messages. An edit sets `edited_at` and keeps no old
  versions, because chat is not the record.
- An admin can remove any message. The author and a "fjernet av administrator" marker stay, the
  text goes, and the removal is audited. Ordinary messages are not audited.
- While the FAU is frozen (a deletion request in progress), posting stops and reading continues.

### 5.4 Delivery

1. The member posts through htmx; the server inserts the message and issues a Postgres `NOTIFY`.
2. Every app instance's `LISTEN` pushes `{thread_id, seq}` over the per-FAU SSE stream (ADR-003
   decision 5a), filtered per viewer by section 3.3.
3. The client fetches everything after its last `seq`, decrypted inside the session. Opening a
   thread loads the last 50 messages and fetches more on scroll.

No new infrastructure is needed, and it works with several app instances.

**Unread markers:** `read_markers(membership_id, thread_id, last_read_seq)` drives the badges. It
is also what a future digest (D7) would read, so adding the digest needs no migration.

### 5.5 Expiry by key epoch

- Chat bodies are encrypted under a **per-FAU monthly chat key** held in the key service,
  wrapped by the FAU's KEK. The long-lived FAU data key is not used for them.
- A daily job deletes messages older than 12 months and **destroys that month's key**, so copies
  in database backups become unreadable too. This is what makes the expiry true rather than
  merely tidy.
- ADR-003 makes queued key destruction a critical alert. Routine epoch expiry is a **separate,
  expected class**: silent when it destroys an epoch at least 12 months old, **critical when
  anything tries to destroy a younger chat key**. This goes to #3442.
- **Article 17:** the person's message bodies are deleted and their name becomes "Tidligere
  medlem". Backup copies become unreadable within 12 months, when the epoch key is destroyed. The
  privacy notice says so.

## 6. Calendar

### 6.1 Model

An event is the FAU's record, kept like documents. Only its thread expires, under section 5.5.

`events`:

- `group_id` for the audience (section 3.2).
- `kind`: `meeting` | `happening` | `deadline`.
- **Times:** either `starts_at` / `ends_at` as `timestamptz`, or all-day `starts_on` /
  `ends_on_exclusive` as local dates. They are always shown in Europe/Oslo, and a check ensures
  that exactly one form is filled.
- **Encrypted** `title`, `location` and `description`, the description in the section 5.2 format.
- `visibility`: `internal` | `public`. **Both values are allowed from the first migration, but
  the application refuses `public`** until publishing exists (D6).
- `revision`, bumped on every change; `cancelled_at`; `created_by`.
- Creating, editing and cancelling an event are audited.

**A meeting is an event** with `kind = meeting` (prosjektgrunnlag §9). The notice, agenda and
minutes documents carry `event_id` as an anchored application row, so the event is the meeting's
identity and there is no second "meeting" concept. **This binds #3419's document migration.**

**Repeats:** there are no recurrence rules in the MVP. "Gjenta" creates N independent copies
(weekly, every other week or monthly), and "Dupliser" copies one. A real series, meaning RRULE
with exceptions, multiplies the complexity of both editing and the feed.

### 6.2 Screens

- A month grid and an agenda list, filtered by group, rendered with htmx. Each event has its
  thread. Cancelled events stay visible, struck through.
- There is no week view and no RSVP in the MVP. For meetings, attendance belongs in the minutes;
  RSVP for happenings is a candidate for later.
- While the FAU is frozen, no events can be created or edited; reading and downloads continue.

### 6.3 Subscription feed

- **URL:** one feed per membership, at `/cal/<token>.ics`. The token is 256 random bits and is
  stored hashed. The member can revoke and regenerate it, and it dies with the membership.
- **Who sees what:** the feed lists the events that member may see at the moment of the fetch
  (section 3.3), so a guest's feed shows only their groups.
- **Contents:**
  - `SUMMARY`: a generic, localised label per kind: "FAU-møte", "FAU-arrangement" or "FAU-frist";
  - start and end;
  - `STATUS:CANCELLED` when cancelled;
  - a stable `UID`, and `SEQUENCE` set from `revision`;
  - `URL`: a link back into the app;
  - `VTIMEZONE` for Europe/Oslo.
- **Never in the feed:** title, location, description, FAU name or school name. The school name
  would tell Google or Apple which school the member's child attends.
- **The feed handler has no access to the key service.** No decryption happens outside a session
  (ADR-003 decision 5a), and nothing sensitive sits with US calendar providers.
- **The token is in the URL path, so the access log must redact that path segment.** The existing
  rule covers query strings only. The feed is rate-limited per token.
- Google refreshes subscriptions roughly every 8–24 hours, so the UI says changes can take up to a
  day to appear.

### 6.4 "Legg til i min kalender"

- A single-event `.ics` file, generated inside the session, with full details. Formatting markers
  are stripped, and document links are written as URLs.
- It gets a **different `UID` from the feed**, so a member who both subscribes and downloads gets
  two independent entries, not a UID clash. The UI explains the difference in one line.

## 7. Date poll

### 7.1 Tables

- `polls`: `group_id` for the audience, `organiser_membership_id`, encrypted `title` and
  `description` (section 5.2 format), `status` (`open` | `closed` | `decided`), an optional
  `respond_by` date, `decided_option_id` and `event_id`.
- `poll_options`: candidate times using the same time type as events, at most 20 per poll.
- `poll_votes`: `(poll_id, option_id, membership_id, answer)`, where `answer` is `yes`,
  `if_need_be` or `no`. No row means no answer.

### 7.2 Behaviour

- **Who votes:** anyone with write access to the audience can create a poll and vote. A reader
  without write access, such as a member viewing an open group's poll, sees the grid but cannot
  vote.
- **Votes are open:** a grid of people × options, with a total for each option.
- `respond_by` is informational only. Nothing closes automatically (D8).
- **Deciding:**
  - The organiser, or any admin (in case the organiser has left), closes the poll and picks an
    option.
  - Picking creates the event: the audience, title and description are copied, and the kind
    defaults to `meeting` but can be changed.
  - The poll then shows "Avgjort" with a link to the event, and a system message is posted in its
    thread.
  - Before deciding, the poll can be closed and reopened freely. After deciding, the event is what
    gets edited.
- **Editing options once votes exist:** adding an option is free; removing one warns first and
  deletes its votes.
- Vote changes send `{poll_id, revision}` over the SSE stream, with the same per-viewer filter.

### 7.3 Retention

- A poll is coordination. **The whole poll (options, votes and thread) is deleted 12 months after
  it closes.** The event it created remains the record.
- Votes are **not encrypted**. An answer enum next to a membership id is the same kind of metadata
  as ids and timestamps under ADR-003 decision 6. Title and description are content and are
  encrypted.
- The honest consequence: vote rows can outlive the purge in database backups until those backups
  age out. The privacy notice states the backup retention period.

## 8. What this binds elsewhere

| Card | What it gains |
|---|---|
| #3412 remainder | `organization_unit`, `cohort` and `unit_cohort` migrated before groups; the open foreign keys on `roles` close |
| #3418 | An audit of read paths for "active membership = read all" before `Guest` is added; invitations can grant a guest role naming a group; display name captured when an invitation is accepted |
| #3419 / #3490 | Folders with an audience `group_id`, documents inheriting access from their folder, and documents anchored to an `event_id` |
| ADR-003 / key service | A new key class for monthly chat epoch keys (wrapped by the KEK); expiry by destroying epochs; the concurrency ceiling still counts per FAU |
| #3442 | Destroying a chat epoch key younger than 12 months raises a critical alert; routine epoch expiry raises nothing |
| Logging (app-foundation design) | The feed token's path segment is redacted, alongside query strings |
| #3439 | All new strings, including the feed labels, are Bokmål source strings; collation is per locale |
| Privacy notice and DPA | Names kept for history after membership ends; contact emails visible to the FAU; the 12-month chat expiry and its backup tail; the backup tail for poll votes |

## 9. Deferred

- The email digest (a paid add-on per FAU), web push and mentions.
- Direct messages, which are ruled out rather than deferred (D2).
- The public calendar (the column exists) and RSVP for happenings.
- Recurrence rules, a week view and a group lead role.
- Per-group encryption keys.
- Chat attachments, which are replaced by document links.
- A live preview in the chat composer (Erik on #3500: possibly later).

## 10. Tests

Each piece's plan expands these; they are listed here so none is lost.

- **Authorization matrix:** a single table-driven test over viewer class × group visibility ×
  in/out of the group × resource type (thread, message, event, poll, folder, directory entry). A
  new resource type must add rows to it.
- **SSE:** no event reaches an unauthorized viewer, including a guest next to a closed group.
  Revocation closes a live stream.
- **Directory:**
  - the mailto length guard at the boundary;
  - a person selected through two groups appears once;
  - the name survives the end of a membership while the contact email disappears;
  - erasure replaces the name;
  - an audit entry records the count and never the addresses.
- **Chat:**
  - `seq` stays gapless under concurrent posts;
  - the parser is fuzzed with `<script>`, unbalanced markers and `javascript:` links;
  - document links render correctly for a guest, for a member outside a closed group and for a
    purged document;
  - a document link to another FAU is rejected on write;
  - a message is unreadable after its epoch key is destroyed;
  - destroying a young epoch key raises a critical alert;
  - an admin removal keeps the marker and drops the text.
- **Calendar:**
  - an event across the October daylight-saving change;
  - all-day and timed events;
  - the feed handler cannot reach the key service;
  - the feed follows guest and closed-group rules at fetch time;
  - a revoked token returns 404;
  - the token never appears in logs;
  - `public` visibility is refused;
  - `.ics` output passes a validator.
- **Poll:**
  - deciding creates exactly one event, even with a double submit;
  - an admin can decide after the organiser's membership ends;
  - a guest votes only in their own groups;
  - removing an option removes its votes;
  - the 12-month purge keeps the event;
  - a reader without write access gets 403 when voting.
