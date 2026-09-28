# The Member Directory: Implementation Plan (#3502)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The storage, rules and reads behind the member directory: a display name captured when a person joins, an optional contact address, both encrypted and bound to their membership; a directory read that shows each viewer exactly the people the group rule lets them see and what each represents; a `mailto:` link or copy text for a selection, with every export audited as a count; and the retention rules. Under Erik's **D3** (28 September 2026), neither the name nor the contact address outlives the membership. History shows an ended membership as its role and year ("Leder 2025–2026"), computed at read time.

**Architecture:**
- **Schema.** Migration `0008` adds `encrypted_display_name`, `encrypted_contact_email` and `name_erased_at` to `memberships`. It has envelope checks, checks that a revoked membership holds neither a name nor a contact address (the name check is added by Task 3b, for D3), and a check that an erased membership holds neither field.
- **Domain (pure).** `fau_domain::directory` has four modules. `address` removes repeated recipients and builds the RFC 6068 `mailto:` link, with the To/Bcc default and the length guard, and the copy text. `collation` orders decrypted names by locale through ICU4X. `listing::arrange` orders the decrypted directory for one viewer's locale. `history::role_label` picks the role and years that history shows for an ended membership.
- **Persistence.**
  - `prepare_acceptance` tells the caller which FAU and membership an acceptance will write, so it can encrypt the profile before `accept_invitation` stores it.
  - `set_display_name` and `set_contact_email` edit the fields.
  - `member_directory` reads the sections a viewer may see. It uses the same facts and rule as `list_groups` and the same member SQL as `list_group_members`.
  - `export_addresses` hands over a selection's addresses and audits it.
  - `member_names` resolves history: a name while the membership is active, the roles it held once it has ended, and "Tidligere medlem" after an erasure.
  - `clear_ended_profiles` and `erase_member_names` apply retention.
- **The session** (#3417, not built here) decrypts names and addresses under the FAU's record key, calls `arrange`, and builds the link from the domain.

**Tech Stack:** Rust 1.98.1, sqlx 0.8 on PostgreSQL 17, jiff, serde_json, `fau-crypto`'s XChaCha20-Poly1305 envelope under `fau-<tenant>-record` (the data-key flow in `fau-keys`), and **ICU4X `icu_collator` 2.3** with `icu_locale_core` 2.3 for collation.

**Spec:** `docs/groups-directory-chat-calendar-design.md` (accepted 26 September 2026 on #3500). This plan implements §4 in full, the directory rows of §8 and §10, and builds on what #3501 built for §3. It also draws on `docs/localisation-design.md` (#3439: Bokmål source strings, per-locale collation, more than two locales), `docs/identity-and-encryption.md` §6, §6a and §7a (retention, and Article 17 as a separate request), and `docs/key-service-design.md` §3.1–3.2 (record key, AAD, data-key flow). Executors read spec §4 before Task 4.

## Global Constraints

- **Every tenant table carries `tenant_id`, and every reference is composite.** `0008` adds columns to `memberships` only, and no foreign key. `schema_review.rs` must stay green.
- **No key material in any application table.** Column names must not contain `key`, `dek`, `kek`, `secret`, `private`, `passphrase`, `password`, `cipher`, `nonce` or `wrapped`. The new columns are `encrypted_display_name`, `encrypted_contact_email` and `name_erased_at`.
- **Names and contact addresses are content.** The caller encrypts each under `fau_crypto::Unit::Record { tenant }` with `Aad::new(tenant_id, "memberships", "encrypted_display_name" | "encrypted_contact_email", membership_id)` (`DISPLAY_NAME_AAD` and `CONTACT_EMAIL_AAD`). Persistence accepts and returns only `fau_crypto::Ciphertext`.
  - A name or an address never appears in audit parameters, a NOTIFY, a log line or a `Debug` output.
  - The login address (`accounts.email`) stays plaintext (ADR-003 §6).
- **No system mail is ever sent to a contact address** (§4.1). Login, invitations and recovery keep using `accounts.email`.
- **Every read path authorizes inside its own snapshot.** `member_directory` and `member_names` run in `read_transaction`. `export_addresses` runs in a REPEATABLE READ transaction that also writes its audit entry.
- **Hidden looks like not found.** A group the viewer cannot read gets `UnknownGroup`, exactly as for an id that does not exist. A person outside the viewer's scope gets `UnknownMembership`, whether or not the id exists elsewhere.
- **Never reveal state through the order of refusals.** Order: input validation, tenant state, then authority, then row state. A non-admin naming someone else gets `NotAuthorized` whether or not the id exists.
- **Ids are UUIDv7.** A new membership's id is chosen before encryption so that the AAD can bind to it: `prepare_acceptance` for invitations, and the caller on activation.
- **Dates cross the SQL boundary as text**, and rule-deciding time comes from the caller's `Moment` (`membership/sql.rs`).
- **Migrations continue from `0007` on main.** `0008` inserts `schema_contract` version 8. `MINIMUM_CONTRACT_VERSION` stays 2 (Ruling R21). Never edit an applied migration. Task 3b amends `0008` in place for D3, after a read-only check that no persistent database has applied it. If one has, add `0009` instead.
- **Collation is never byte order** (§4.3, #3439). Nothing may hard-code two locales or assume Latin collation.
- **English** for all technical text, code, comments and commit messages. #3502 renders **no user-facing strings**. The Bokmål source strings the screen will need are recorded as catalogue entries (Ruling R20). Never author Nynorsk.
- **Test commands** run from `/workspace/backend` with these variables exported:
```bash
cd /workspace/backend
export TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres
export TEST_OPENBAO_ADDR=http://openbao:8200
export TEST_OPENBAO_TOKEN=dev-only-root
```
  The app stack is `docker compose -f /workspace/compose.yaml` (project `fau-app`), never `/workspace/docker-compose.yml`. Default test parallelism is fine. **Never run two workspace test runs at once** (Postgres allows 100 connections): check `ps aux | grep '[c]argo test'` first. If `db` does not resolve, follow the memory note "agent reaches app DB", and warn Erik first, because attaching reloads the terminal.
- **Before every commit:** `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` must be clean.
- **Commits** go on branch `directory-3502`. Every message ends with `(#3502)`, a blank line, and the committing agent's own `Co-Authored-By` trailer. The trailer shown in each commit step is the planner's; use your own. If git has no identity, pass `GIT_AUTHOR_NAME="Erik W. Bjønnes" GIT_AUTHOR_EMAIL=erik@ewb-solutions.as` and the same `GIT_COMMITTER_*` on each command. Never change git config and never push.
- **Security tests must be able to fail.** Each authorization test includes:
  - cross-tenant cases;
  - a guest outside the group;
  - revoked, ended and disabled standing;
  - a positive control.

  Each such test names the mutation it catches. Five of them were mutation-checked while the plan was written (see Self-review).

## Plan decisions and rulings (for Erik's review)

The spec leaves these open, and the plan decides them. Each carries its reason and the cost if it is wrong.

- **R1: #3502 builds the schema, the domain and persistence. The screen and its routes wait for #3417.**
  - What is not built: the htmx screen, its routes, the TypeScript selection module and the profile edit form. The app has no sessions yet, so nothing identifies a viewer or holds a record key. An unauthenticated directory route would expose every member's address. A route nothing mounts would fail `clippy -D warnings`.
  - What the screen card gets is listed under "What later cards get from #3502".
  - Cost if wrong: none here. The screen card adds routes over these functions.
- **R2: The fields are three columns on `memberships`, not a new table.** The spec makes both fields per membership, and a membership row already has the lifecycle they follow (reopen on re-invite, revoke).
  - `encrypted_display_name` is nullable in the schema for two reasons: rows created before `0008` exist (the dev database and SQL fixtures), and an erasure removes the name. The code requires a name on every membership it creates (R3, R4).
  - A `not valid` check was rejected: PostgreSQL enforces such a check on every later update of an old row, so revoking a pre-`0008` membership would fail.
  - Cost if wrong: a one-table move.
- **R3: Acceptance takes two calls, because the AAD binds a name to its membership id.**
  - `prepare_acceptance(token, verified address)` returns the FAU (for the record key) and the membership id: the existing one for a re-invited person, or a fresh UUIDv7.
  - The caller encrypts, then calls `accept_invitation` with `AcceptInvitation.profile`.
  - If the membership found at acceptance has any other id, the call fails with `AcceptanceTargetChanged`. That happens when a concurrent acceptance created it. A profile therefore never lands on a row its AAD does not name; the caller prepares again and retries once.
  - `prepare_acceptance` answers for any invitation whose recipient is the verified address, pending or not, so that `accept_invitation` can give the precise refusal ("expired"). Everything else gets `UnknownInvitation`.
  - Rejected: letting persistence call back into encryption. `fau-keys` depends on `fau-persistence`, and holding the tenant lock across an OpenBao call is worse.
  - Cost if wrong: a single-call accept would need the key service inside persistence.
- **R4: The registrant's name is captured at activation, by the same rule.** `Activation.profile` names a fresh membership id, since a pending FAU has no memberships. The spec names only acceptance, but otherwise the FAU's first admin would be the one nameless member.
- **R5: A new acceptance replaces the name and the contact address.** A re-invited former member states the name that applies from now on. *Amended by D3-2:* the old row holds no name by then, because revocation or the sweep cleared it. Only a member whose roles ran out before the sweep reached them has a name replaced.
- **R6: Who edits what.**
  - A member edits their own name, a guest included.
  - ~~An admin corrects anyone's name, including a former member's, since history shows names.~~ *Replaced by D3-6:* an admin corrects the name of anyone whose membership is still active. An ended membership takes no name.
  - Only the member sets or clears their contact address, because it is their own statement; an admin cannot.
  - Name edits and setting an address are refused while the FAU is frozen. Clearing an address is allowed, because it reduces what others see.
  - Every edit needs standing today.
- **R7: No name history.** *Superseded by D3-1*, which keeps the conclusion for a stronger reason. An edit overwrites the name. The audit entry `membership.display_name_changed` records that it changed and whether an admin did it, never the value.
- **R8: When a membership ends, and so does its contact address.** *Superseded by D3-2*, which keeps this definition of "ended" and extends every clearing below to the name. A membership has ended when it is revoked, or when none of its role assignments is still running or yet to start.
  - Revocation clears the address in the same statement. Migration `0008`'s `memberships_contact_email_only_while_current` enforces this, so no code path can forget.
  - A membership whose roles simply ran out is not revoked. `clear_ended_contact_emails` clears it: a sweep per FAU, under the tenant lock and re-checked under it, with each clearing audited by the system actor.
  - It is cleared at once, not after the account's three-month grace. That grace protects re-recognition, and re-entering an address costs a second.
  - Cost if wrong: add a date offset to one fragment.
- **R9: Article 17 erasure is built as a storage step only.** *Amended by D3-4:* the marker now also suppresses the role-and-year label.
  - `erase_member_names(account)` removes both fields from every membership the account holds, marks each erased and audits it without the name. #3426 builds the flow around it: who asks, what else goes, and whether the membership ends.
  - An erased person is not listed in the directory. `member_names` returns `Former`, which renders as "Tidligere medlem".
  - An erased membership cannot be accepted into again (`MembershipErased`), because a new name on the old row would re-attach history to it.
  - Database backups keep the old ciphertext until they age out; only deleting the FAU shreds it. This goes in the privacy notice.
- **R10: `member_names` serves history, for members and admins only.** A guest gets `NotAuthorized` until the first feature that shows a guest history (#3503 chat authors) decides what a guest may resolve. Ids outside the viewer's FAU are left out. D3 leaves this unchanged.
- **R11: What the directory shows** (§4.3, §4.4, §3.2).
  - It has one FAU-wide section, for members and admins only, that lists every current member and admin and never a guest.
  - It has one section per group the viewer may read, through `readable_groups`, the facts and rule `list_groups` uses, listing members through `group_members_sql`, the SQL `list_group_members` uses. Guests appear inside those sections.
  - "Current" means standing today: a usable membership with a role assignment valid today. A handover grant alone does not count.
  - "What a person represents" is:
    - every role assignment valid today: role name, plus unit or cohort;
    - the listed groups the person is in.
  - A group the viewer cannot read is never shown as part of a person, so a closed group's membership is not disclosed through the directory.
- **R12: The "Gjest" marker** means the person holds only guest-class roles today.
- **R13: Collation uses ICU4X (`icu_collator` 2.3), sorting after decryption, in the domain.**
  - Why ICU4X: it is the Unicode Consortium's own library, it is used in Firefox, and it is already most of the way into `Cargo.lock` through `url`'s IDNA support. It adds 6 crates, all from ICU4X or its support libraries: `icu_collator`, `icu_collator_data`, `icu_locale_fallback`, `icu_locale_fallback_data`, `utf16_iter` and `write16`. This follows Erik's rule to prefer battle-proven components.
  - Rejected: `feruca`, which is UCA without per-locale tailoring and puts Å with A, which is wrong for Bokmål. Binding to system ICU was rejected because the runtime image would need `libicu`.
  - A tag that does not parse falls back to `nb-NO`. Ties are broken by id. Unnamed rows sort last.
  - **Data gap:** ICU4X's compiled data has CLDR's `nb` and `nn` tailoring but not Sámi (`se`, `sma`, `smj`), which fall back to the root order. This was checked while planning. Adding a Sámi locale means generating ICU4X data that includes it, behind the one constructor `NameCollator::for_locale`. Erik's D4 (28 September): the data is added when a Sámi locale is added.
- **R14: Address formats.**
  - `mailto:` joins addresses with a bare `,`, because RFC 6068's grammar has no whitespace, and `, ` would cost `%20` per address against the length guard.
  - Every byte except the unreserved characters and `@` is percent-encoded.
  - A local part that is not a dot-atom is quoted, so a `,` or `;` inside an address cannot split it.
  - The copy text joins with `, ` by default (`CopySeparator::Comma`), with `CopySeparator::Semicolon` ready. The Outlook desktop check that §4.3 asks for belongs to the screen card, since only a running screen can be tried in Outlook.
- **R15: The length guard.** A link longer than 1,800 characters, counting the final encoded URL, is `Mailto::TooLong { length }`. Exactly 1,800 is still a link.
- **R16: Bcc above 10.** The default is Bcc when more than 10 distinct addresses remain after repeats are removed. Repeats are removed twice: by person in persistence, and by address in the domain, since two people may share one.
- **R17: Every export goes through the server and is audited.**
  - `export_addresses(purpose, scope, ids)` is the only way addresses leave for a link or a copy.
  - Its audit entry `directory.addresses_exported` holds exactly `{purpose, recipient_count, scope, group_id}`.
    - `recipient_count` counts distinct people.
    - The entry holds no addresses and no membership ids.
    - `scope` is the screen's group filter (`all`, `fau` or `group` with its id). A list of groups was not used, because it could outgrow `audit_params_are_small`'s 2,048 bytes.
  - Allowed while frozen, since reading continues.
  - Showing the addresses on the directory screen itself is not an export. The spec lists them on every entry.
- **R18: Refusal order in the export.**
  1. `EmptySelection`.
  2. `NotAuthorized`: no standing, or a guest asking for the FAU-wide section.
  3. `UnknownGroup`: for any group the viewer cannot read.
  4. `UnknownMembership`: for any selected person the scope does not list.

  A refusal writes nothing.
- **R19: Directory edits send no change notifications.** No `Resource` names a membership, and the screen reloads after an edit. Cost if wrong: one `notify` per edit, once a membership resource exists.
- **R20: No rendered strings and no new `ErrorCode`.** #3502 has no HTTP surface. The screen card adds these Bokmål source strings to the catalogue (`nb-NO.json`, #3439). Keys are proposals:

  | Key | Bokmål source |
  |---|---|
  | `directory.section.fau` | "Hele FAU" |
  | `directory.guest` | "Gjest" |
  | `directory.unnamed` | "Navn ikke oppgitt" |
  | `member.former` | "Tidligere medlem" |
  | `member.endedRole.oneYear` | "{role} {year}" (D3-5) |
  | `member.endedRole.years` | "{role} {firstYear}–{lastYear}" (D3-5) |
  | `directory.selectAll` | "Velg alle" |
  | `directory.compose` | "Skriv e-post" |
  | `directory.copy` | "Kopier adresser" |
  | `directory.field.to` / `directory.field.bcc` | "Til" / "Blindkopi" |

  The length-guard explanation and the guest-invitation sentence ("you will be reachable by the group", §4.4) are the screen card's to word. The mapping of `EmptySelection`, `MembershipErased` and `AcceptanceTargetChanged` to error codes is #3417's.
- **R21: Schema contract.** `0008` is version 8, and the minimum stays 2. The migration only adds nullable columns and checks that existing rows satisfy.
- **R22: A contact address is parsed with `Email::parse` before it is encrypted**, with no extra rule. The builders handle any `Email` safely (R14).
- **R23: `DirectoryPerson.is_viewer`** marks the viewer's own entry, where the screen offers editing.

### Rulings (D3), 28 September 2026

Erik's decision D3: *"Display names are only valid when active members, when they are no longer active we change from display name to role and year, which helps keep things GDPR valid."* It overrides spec §4.2 and Rulings R7–R9. These rulings apply it where D3 leaves the detail open.

- **Ruling (D3): D3-1, no name history (replaces R7).** A display name exists only while the membership is active, so there is no past name to keep, and an edit overwrites. Open question 1 is closed.
  - Reason: D3 removes the name when the membership ends, and history uses role and year instead.
  - Cost if wrong: a later `membership_names` table, the same cost as before.
- **Ruling (D3): D3-2, an ended membership holds neither field (replaces R8).** "Ended" keeps R8's meaning: revoked, or no role assignment still running or yet to start. `membership_ended` in `profile.rs` is the one SQL definition, shared by editing, the sweep and history.
  - Revocation clears the name and the contact address in the statement that revokes. Migration `0008`'s `memberships_display_name_only_while_current` (Task 3b) and `memberships_contact_email_only_while_current` refuse any path that forgets.
  - A membership whose roles ran out is cleared by the daily `clear_ended_profiles` (renamed from `clear_ended_contact_emails`), audited as `membership.profile_cleared {cause: "membership_ended"}`. It is cleared at once, with no three-month grace.
  - A handover grant does not keep a membership going: it is a recovery right, not a role. A former admin in the handover window is shown by role and year.
  - A membership whose roles are all still to come is active and keeps its name.
  - Reason: D3, and "ended" was already defined for the contact address.
  - Cost if wrong: one fragment changes, for example to add a grace period.
- **Ruling (D3): D3-3, "ended" and the label are computed at read time, never stored.** `member_names` decides "ended" at the moment of the read, from `role_assignments`, so a sweep that has not run yet never lets a name through. It then returns the roles the membership held (`MemberName::Ended(Vec<HeldRole>)`). No label column exists.
  - Reason: the role facts are already kept as plaintext school structure (ADR-003 decision 6), and a stored label could drift from them.
  - Cost if wrong: a renamed role renames old labels too, and history loses its labels if role assignments are ever purged. An ended membership with no held role shows "Tidligere medlem" (`Former`).
- **Ruling (D3): D3-4, an Article 17 erasure shows "Tidligere medlem", never role and year (amends R9).** `name_erased_at` keeps its name and gains a role: an erased membership, active or ended, renders as `Former`. It still cannot be accepted into again (`MembershipErased`), and the directory still leaves it out.
  - Reason: a single-holder role with its year ("Leder 2025–2026") identifies the person as well as a name does, and an erasure is a request to stop identifying them.
  - Cost if wrong: erased people lose role context in history. The assignments remain, so #3426 can revisit this.
- **Ruling (D3): D3-5, which role and which years.** `fau_domain::directory::history::role_label(held, event_date)` decides:
  - the role held on the day of the event, so that a minute from someone's year as Kasserer says Kasserer even if they later led the FAU;
  - among roles held that day: admin class, then member, then guest, then the earliest start;
  - after every role has ended: the last role that ended; before any role began: the first role to begin;
  - one role, never a list, and no unit or cohort, because a place narrows a role down to a person;
  - years: the calendar years of the first and last day the assignment was actually held, where an assignment revoked early ends on its revocation day (Oslo date). An assignment revoked before it started was never held.
  - Rendering: `member.endedRole.oneYear` "{role} {year}" when both years are equal, otherwise `member.endedRole.years` "{role} {firstYear}–{lastYear}". Both are proposed catalogue strings (R20), with the years passed as strings so ICU does not group their digits. #3502 renders nothing.
  - Reason: D3's example "Leder 2025–2026", applied truthfully to people who held several roles.
  - Cost if wrong: the rule lives in one pure function with unit tests; school-year arithmetic (1 August) would be a local change there.
- **Ruling (D3): D3-6, no edits on ended memberships (replaces R6's former-member clause).** `set_display_name` refuses an ended target with the new `MembershipError::MembershipEnded`. The check comes after authority, so only an admin, or the member acting on themself, learns it.
  - Setting a contact address still needs standing today (unchanged). A handover-grant holder without a role could set one: it is shown nowhere, since the directory needs a role valid today, and the sweep clears it. This is accepted rather than coded.
  - Reason: a name on an ended membership would contradict D3, and on a revoked row the database refuses it anyway.
  - Cost if wrong: admins cannot correct how a former member is shown. Their label comes from the role, which admins can edit.
- **Ruling (D3): D3-7, acceptance and activation are unchanged.** A name is still required while the membership is active. A re-invited former member reopens their row, which holds no name, and states one again (R5 as amended).
- **Ruling (D3): D3-8, the directory read and the export are unaffected.** Both list only people with standing today. Standing today implies an active membership, so every listed name is valid under D3 and no listed entry is `Ended` or `Former`. Task 7 only gains a doc line saying so, and Task 8's code is unchanged; only its `error.rs` hunk context moved.
- **Also answered 28 September:** D4 (Sámi collation data is added with a Sámi locale; R13) and D5 (archived groups stay out of the directory, confirming execution ruling Q6).

**Open questions for Erik:** none remain from this plan. Question 1 (name history) is closed by D3, and question 2 (Sámi collation data) by D4.

**Spec against code:**
- ADR-003 §6 lists "member emails and names" as plaintext. The accepted spec §4.1 and key-service §3.1, both later, encrypt display names and contact addresses. The later documents win. The login address stays plaintext.
- Spec §4.3 says addresses are "joined with `, ` per RFC 6068", but RFC 6068's `mailto:` grammar has no space. Ruling R14 applies `, ` to the copy text and `,` to the link.
- Spec §8 gives "display name captured when an invitation is accepted" to #3418. #3501's plan handed it to #3502, and it is Task 4 here.
- Spec §4.1 makes the name required, but pre-`0008` rows have none. R2 handles them, and `ShownName::Unnamed` renders them.
- **Spec §4.2 says "the name stays after the membership ends".** Erik's D3 (28 September 2026) overrides that: the name goes with the membership, and history shows role and year. The Rulings (D3) below apply it. The spec's text is not edited here; the #3500 document owner updates §4.2.

## What later cards get from #3502, and nothing more

- **The screen card (#3417 sessions, #3422 app shell).**
  - The directory page:
    1. `member_directory(viewer)`;
    2. `data_key(.., Unit::Record { tenant })` once per session;
    3. decrypt the group names (`GROUP_NAME_AAD`), display names (`DISPLAY_NAME_AAD`) and contact addresses (`CONTACT_EMAIL_AAD`), each bound to its row id;
    4. `arrange(sections, people, &NameCollator::for_locale(resolved_locale))`;
    5. render.

    `tests/key_chain.rs`'s last test is that route without HTTP.
  - "Skriv e-post" and "Kopier adresser":
    1. `export_addresses(viewer, AddressExport { purpose, scope: the active filter, membership_ids: the ticked ids })`;
    2. decrypt;
    3. `Recipients::new(..)`;
    4. `.mailto(field)` or `.copy_text(separator)`.

    The server's answer is authoritative. The TypeScript module tracks the ticks, removes repeats, and starts the To/Bcc toggle at `BCC_ABOVE`. It may pre-compute the link length from the addresses already on the page to grey out the button, porting `address_spec` and the percent-encoding. It must use the domain tests' vectors as its own tests.
  - Accepting an invitation:
    1. `prepare_acceptance`;
    2. `data_key`;
    3. encrypt the typed name (`DisplayName::parse`) and the optional address (`Email::parse`);
    4. `accept_invitation`, retrying from step 1 once on `AcceptanceTargetChanged`.

    Activation is the same with a fresh `Uuid::now_v7()`.
  - Profile edits: `set_display_name` and `set_contact_email`. An admin correcting someone else's name is a privileged admin action, so it passes #3414's gate first. A member's own edit needs no gate. `MembershipEnded` means the membership has ended and takes no name (D3-6).
- **The job runner** (with `lapse_requests` and `create_handover_grants`) schedules `clear_ended_profiles` daily.
- **#3426 (Article 17):** `erase_member_names(account_id)` is its storage step (R9, D3-4).
- **#3503 (chat) and every history renderer** resolve people through `member_names`, then render:
  - `Named`: decrypt with `DISPLAY_NAME_AAD`;
  - `Unnamed`: "Navn ikke oppgitt";
  - `Ended(held)`: `role_label(&held, event_date)`, using the date of the message, minute or audit line, rendered with `member.endedRole.oneYear` or `member.endedRole.years`;
  - `Former`: "Tidligere medlem".

  A renderer never caches a name beyond the request, since it goes when the membership ends. #3503 decides the guest rule (R10).
- **#3421 (audit view)** renders `directory.addresses_exported`, `membership.display_name_changed`, `membership.contact_email_changed`, `membership.profile_cleared` and `membership.name_erased` from their codes.
- **Privacy notice and DPA** (§8):
  - names and contact addresses are deleted when the membership ends;
  - afterwards history shows the role and year, from the FAU's role records;
  - contact addresses are visible to the FAU;
  - an erasure shows "Tidligere medlem" and has a backup tail.

## File Structure

```
backend/
  Cargo.toml                                       modify: icu_collator, icu_locale_core in [workspace.dependencies]
  Cargo.lock                                       regenerated by cargo (Task 3)
  migrations/0008_member_directory.sql             NEW  the three columns and five checks (Task 3b amends it in place)
  crates/domain/
    Cargo.toml                                     modify: icu_collator, icu_locale_core, uuid
    src/lib.rs                                     modify: pub mod directory
    src/membership/vocabulary.rs                   modify: DisplayName
    src/directory/mod.rs                           NEW  module doc
    src/directory/address.rs                       NEW  Recipients, mailto, copy text, length guard
    src/directory/collation.rs                     NEW  NameCollator (ICU4X)
    src/directory/listing.rs                       NEW  SectionId, Person, arrange
    src/directory/history.rs                       NEW  HeldRole, RoleLabel, role_label (D3)
  crates/persistence/src/membership/
    mod.rs                                         modify: modules and re-exports
    error.rs                                       modify: six variants
    sql.rs                                         modify: ensure_membership takes the expected id
    profile.rs                                     NEW  AADs, MemberProfile, membership_ended, set_display_name, set_contact_email
    invitations.rs                                 modify: AcceptInvitation.profile, prepare_acceptance
    signup.rs                                      modify: Activation.profile
    roles.rs                                       modify: revocation clears the name and the contact address
    names.rs                                       NEW  MemberName, member_names
    retention.rs                                   NEW  clear_ended_profiles, erase_member_names
    groups.rs                                      modify: readable_groups, group_members_sql shared
    directory.rs                                   NEW  member_directory and its types
    export.rs                                      NEW  export_addresses
  crates/app/tests/
    common/membership.rs                           modify: placeholder_envelope, fresh_profile, profile_for
    outbox.rs guests.rs invitations.rs requests.rs signup.rs   modify: pass a profile
    handover_recovery.rs                           modify: pass a profile; the re-invite uses profile_for
    invitations.rs roles.rs                        modify: raw-SQL revocation fixtures clear the name (D3)
    directory_schema.rs                            NEW  0008 (Task 3b: the name check)
    member_profile.rs                              NEW  capture at acceptance and activation
    profile_edits.rs                               NEW  edits
    directory_retention.rs                         NEW  sweep, erasure, history's role and year
    authorization.rs                               modify: directory entries in the matrix
    directory.rs                                   NEW  sections agree with group reads; entries
    address_export.rs                              NEW  export and its audit
    key_chain.rs                                   modify: the directory end to end
docs/planning-decisions.md                         modify: record the rulings (Task 9)
```

---

### Task 1: Migration 0008: the directory fields on `memberships`

**Files:**
- Create: `backend/migrations/0008_member_directory.sql`
- Test: `backend/crates/app/tests/directory_schema.rs`

**Interfaces:**
- Consumes: migrations `0001`–`0007`; `common::membership::school(pool, label) -> Uuid`.
- Produces:
  - columns `memberships.encrypted_display_name bytea`, `memberships.encrypted_contact_email bytea` and `memberships.name_erased_at timestamptz`, all nullable;
  - checks `memberships_display_name_is_an_envelope`, `memberships_contact_email_is_an_envelope`, `memberships_contact_email_only_while_current` and `memberships_erasure_leaves_nothing`;
  - `schema_contract` version 8.

- [ ] **Step 1: Write the failing test**

Create `backend/crates/app/tests/directory_schema.rs`:

```rust
//! Migration 0008: the member directory's fields on `memberships` (groups design §4.1,
//! §4.2), proven in SQL before any Rust depends on them.

mod common;
use common::TestDb;
use sqlx::PgPool;
use uuid::Uuid;

fn constraint_name(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.constraint())
        .map(|c| c.to_owned())
}

/// Shaped like fau-crypto's envelope: version byte `version`, then `len - 1` bytes.
fn envelope(version: u8, len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[0] = version;
    v
}

/// One active FAU with one membership, straight in SQL.
async fn seed(pool: &PgPool) -> (Uuid, Uuid) {
    let school = common::membership::school(pool, &format!("dir-{}", Uuid::now_v7())).await;
    let (tenant, account, membership) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(
        "insert into tenants (id, name, status, school_id) values ($1, 'FAU', 'active', $2)",
    )
    .bind(tenant)
    .bind(school)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("insert into accounts (id, email) values ($1, $1::text || '@example.test')")
        .bind(account)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
        .bind(tenant)
        .bind(membership)
        .bind(account)
        .execute(pool)
        .await
        .unwrap();
    (tenant, membership)
}

async fn set(
    pool: &PgPool,
    id: Uuid,
    column: &str,
    value: Option<Vec<u8>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "update memberships set {column} = $2 where id = $1"
    ))
    .bind(id)
    .bind(value)
    .execute(pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn both_fields_hold_only_a_bounded_envelope() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    for (column, constraint) in [
        (
            "encrypted_display_name",
            "memberships_display_name_is_an_envelope",
        ),
        (
            "encrypted_contact_email",
            "memberships_contact_email_is_an_envelope",
        ),
    ] {
        // Positive controls at both bounds, and null.
        set(&pool, m, column, Some(envelope(1, 42))).await.unwrap();
        set(&pool, m, column, Some(envelope(1, 512))).await.unwrap();
        set(&pool, m, column, None).await.unwrap();
        for bad in [
            envelope(1, 41),
            envelope(1, 513),
            envelope(0, 100),
            envelope(2, 100),
        ] {
            let err = set(&pool, m, column, Some(bad)).await.unwrap_err();
            assert_eq!(
                constraint_name(&err).as_deref(),
                Some(constraint),
                "{column}"
            );
        }
    }
}

#[tokio::test]
async fn a_revoked_membership_holds_no_contact_email() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    set(&pool, m, "encrypted_contact_email", Some(envelope(1, 60)))
        .await
        .unwrap();
    set(&pool, m, "encrypted_display_name", Some(envelope(1, 60)))
        .await
        .unwrap();

    // Revoking without clearing the address is refused.
    let err =
        sqlx::query("update memberships set revoked_at = '2026-09-23T10:00:00Z' where id = $1")
            .bind(m)
            .execute(&pool)
            .await
            .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("memberships_contact_email_only_while_current")
    );

    // Clearing it in the same statement is accepted, and the name stays (§4.2).
    sqlx::query(
        "update memberships set revoked_at = '2026-09-23T10:00:00Z', encrypted_contact_email = null
          where id = $1",
    )
    .bind(m)
    .execute(&pool)
    .await
    .unwrap();
    let err = set(&pool, m, "encrypted_contact_email", Some(envelope(1, 60)))
        .await
        .unwrap_err();
    assert_eq!(
        constraint_name(&err).as_deref(),
        Some("memberships_contact_email_only_while_current")
    );
    let name: Option<Vec<u8>> =
        sqlx::query_scalar("select encrypted_display_name from memberships where id = $1")
            .bind(m)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(name, Some(envelope(1, 60)));
}

#[tokio::test]
async fn an_erased_membership_holds_neither_field() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (_, m) = seed(&pool).await;
    for column in ["encrypted_display_name", "encrypted_contact_email"] {
        set(&pool, m, column, Some(envelope(1, 60))).await.unwrap();
        let err = sqlx::query(
            "update memberships set name_erased_at = '2026-09-23T10:00:00Z' where id = $1",
        )
        .bind(m)
        .execute(&pool)
        .await
        .unwrap_err();
        assert_eq!(
            constraint_name(&err).as_deref(),
            Some("memberships_erasure_leaves_nothing"),
            "{column}"
        );
        set(&pool, m, column, None).await.unwrap();
    }
    // Positive control: with both cleared, the erasure marker is accepted.
    sqlx::query("update memberships set name_erased_at = '2026-09-23T10:00:00Z' where id = $1")
        .bind(m)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_runtime_role_can_write_both_fields() {
    let db = TestDb::migrated().await;
    let (_, m) = seed(&db.admin_pool()).await;
    let app = db.app_pool().await;
    set(&app, m, "encrypted_display_name", Some(envelope(1, 60)))
        .await
        .unwrap();
    set(&app, m, "encrypted_contact_email", Some(envelope(1, 60)))
        .await
        .unwrap();
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p fau-app --test directory_schema`
Expected: all 4 tests FAIL, panicking on `column "encrypted_display_name" of relation "memberships" does not exist` (or `encrypted_contact_email`).

- [ ] **Step 3: Write the migration**

Create `backend/migrations/0008_member_directory.sql`:

```sql
-- 0008: the member directory's two fields (#3502; groups design §4.1, §4.2).
--
-- Both are per membership, so one person can go by different names in two FAU-er, and
-- both are content: the backend encrypts them under the FAU's record key with AAD
-- (tenant, 'memberships', <column>, membership id). This table holds the envelopes only,
-- and no key material. The checks are structural, as for groups.encrypted_name in 0007:
-- version byte 1 and 42..512 octets, i.e. 41 bytes of version, nonce and tag around
-- 1..471 bytes of plaintext. A display name (1..100 characters, at most 400 bytes of UTF-8)
-- and an address (at most 254 bytes) both fit with headroom.
--
-- * encrypted_display_name: required by the code when an invitation is accepted or an FAU
--   activated, so every membership created from here on has one. Nullable because rows
--   created before this migration have none, and because an Article 17 erasure removes it.
--   It survives the end of the membership: history shows the names that applied at the
--   time (prosjektgrunnlag §8).
-- * encrypted_contact_email: optional, the member's own statement, never verified and never
--   mailed by the system. It does not survive the end of the membership (§4.2); the check
--   below holds that for revocation, and fau_persistence's clear_ended_contact_emails for a
--   membership whose roles simply ran out.
-- * name_erased_at: an Article 17 erasure replaced the name with "Tidligere medlem" (§4.2).
alter table memberships
  add column encrypted_display_name  bytea,
  add column encrypted_contact_email bytea,
  add column name_erased_at          timestamptz;

alter table memberships add constraint memberships_display_name_is_an_envelope
  check (encrypted_display_name is null
         or (get_byte(encrypted_display_name, 0) = 1
             and octet_length(encrypted_display_name) between 42 and 512));
alter table memberships add constraint memberships_contact_email_is_an_envelope
  check (encrypted_contact_email is null
         or (get_byte(encrypted_contact_email, 0) = 1
             and octet_length(encrypted_contact_email) between 42 and 512));
alter table memberships add constraint memberships_contact_email_only_while_current
  check (revoked_at is null or encrypted_contact_email is null);
alter table memberships add constraint memberships_erasure_leaves_nothing
  check (name_erased_at is null
         or (encrypted_display_name is null and encrypted_contact_email is null));

-- fau_app already holds select, insert, update, delete on memberships (0002).

insert into schema_contract (version) values (8);
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test directory_schema --test migrations --test schema_review --test membership_schema`
Expected: PASS. `directory_schema` shows 4 passed. `migrations` and `membership_schema` see `max(version) = 8 = migration_file_count()`. `schema_review`'s key-material and FK-pairing tests stay green.

- [ ] **Step 5: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 6: Commit**

```bash
git add backend/migrations/0008_member_directory.sql \
        backend/crates/app/tests/directory_schema.rs
git commit -m "Add migration 0008: encrypted display name and contact address on memberships (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `DisplayName`, and the `mailto:` link and copy text

**Files:**
- Modify: `backend/crates/domain/src/membership/vocabulary.rs`
- Modify: `backend/crates/domain/src/lib.rs`
- Create: `backend/crates/domain/src/directory/mod.rs`
- Create: `backend/crates/domain/src/directory/address.rs` (unit tests inside)

**Interfaces:**
- Consumes: `fau_domain::email::Email` (trimmed and lower-cased; `as_str()`); `validate_name` in `vocabulary.rs`.
- Produces:
  - `DisplayName::parse(&str) -> Result<DisplayName, NameError>` (1–100 characters, trimmed, no control characters; redacted `Debug`) and `DisplayName::as_str()`;
  - in `fau_domain::directory::address`:
    - `BCC_ABOVE: usize = 10` and `MAILTO_MAX_CHARS: usize = 1800`;
    - `enum RecipientField { To, Bcc }` with `RecipientField::default_for(count)`;
    - `enum CopySeparator { Comma, Semicolon }` with `as_str()`;
    - `enum Mailto { Link(String), TooLong { length: usize } }`;
    - `Recipients::new(impl IntoIterator<Item = Email>)`, with `len()`, `is_empty()`, `default_field()`, `mailto(RecipientField) -> Mailto` and `copy_text(CopySeparator) -> String`;
    - `address_spec(&Email) -> String`.

- [ ] **Step 1: Write the failing tests**

`DisplayName` joins the redaction and bounds tests of `vocabulary.rs`. The diff below carries the type and its tests: apply its last two hunks (the tests) now, and its first hunk (the type) in Step 3.

```diff
--- a/backend/crates/domain/src/membership/vocabulary.rs
+++ b/backend/crates/domain/src/membership/vocabulary.rs
@@ -169,6 +169,31 @@
     }
 }
 
+/// What a member goes by in one FAU ("Kari Nordmann"), captured when they accept an
+/// invitation (groups design §4.1). Per membership, so one person can go by different names
+/// in two FAU-er. Content, so it is encrypted before it reaches persistence, and `Debug` is
+/// redacted.
+#[derive(Clone, PartialEq, Eq)]
+pub struct DisplayName(String);
+
+impl DisplayName {
+    pub const MAX_CHARS: usize = 100;
+
+    pub fn parse(raw: &str) -> Result<Self, NameError> {
+        validate_name(raw, Self::MAX_CHARS).map(Self)
+    }
+
+    pub fn as_str(&self) -> &str {
+        &self.0
+    }
+}
+
+impl fmt::Debug for DisplayName {
+    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
+        f.write_str("DisplayName([redacted])")
+    }
+}
+
 impl fmt::Debug for GroupName {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
         f.write_str("GroupName([redacted])")
@@ -274,6 +299,10 @@
         assert!(!format!("{:?}", Some(&role)).contains("Kari"));
         assert!(!format!("{fau:#?}").contains("Nordre"));
 
+        let person = DisplayName::parse("Kari Nordmann").unwrap();
+        assert_eq!(format!("{person:?}"), "DisplayName([redacted])");
+        assert!(!format!("{:?}", Some(&person)).contains("Kari"));
+
         let group = GroupName::parse("Oppfølging av sak med rektor").unwrap();
         assert_eq!(format!("{group:?}"), "GroupName([redacted])");
         assert!(!format!("{:?}", Some(&group)).contains("rektor"));
@@ -296,6 +325,18 @@
         );
         assert_eq!(FauName::parse(&"x".repeat(201)), Err(NameError::TooLong));
 
+        assert_eq!(DisplayName::parse(" Kari ").unwrap().as_str(), "Kari");
+        assert!(DisplayName::parse(&"å".repeat(100)).is_ok());
+        assert_eq!(
+            DisplayName::parse(&"å".repeat(101)),
+            Err(NameError::TooLong)
+        );
+        assert_eq!(DisplayName::parse(""), Err(NameError::Empty));
+        assert_eq!(
+            DisplayName::parse("Kari\tN"),
+            Err(NameError::ControlCharacter)
+        );
+
         assert_eq!(GroupName::parse(" Dugnad ").unwrap().as_str(), "Dugnad");
         assert!(GroupName::parse(&"ø".repeat(100)).is_ok());
         assert_eq!(GroupName::parse(&"ø".repeat(101)), Err(NameError::TooLong));
```

Register the module:

Apply to `backend/crates/domain/src/lib.rs`:

```diff
--- a/backend/crates/domain/src/lib.rs
+++ b/backend/crates/domain/src/lib.rs
@@ -2,6 +2,7 @@
 //! `tests/dependency_boundary.rs` enforces that against this crate's own manifest.
 
 pub mod authz;
+pub mod directory;
 pub mod email;
 pub mod error_code;
 pub mod membership;
```

Create `backend/crates/domain/src/directory/mod.rs`:

```rust
//! The member directory (groups design §4; #3502): names, what each person represents and
//! a contact address; select people, then open a `mailto:` link or copy the addresses.
//!
//! Pure. Persistence authorizes and reads the directory (`member_directory`) and audits
//! every export of addresses (`export_addresses`); the session decrypts names and
//! addresses under the FAU's record key; this module orders the result for the viewer's
//! locale and builds the link or the text.

pub mod address;
```

Create `backend/crates/domain/src/directory/address.rs` with its tests module first: the `#[cfg(test)] mod tests { .. }` block at the end of the file below, above an empty implementation section.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p fau-domain --lib directory::address`
Expected: compile errors, `cannot find type Recipients in this scope` and similar. With the vocabulary hunk applied without the type: `cannot find type DisplayName`.

- [ ] **Step 3: Write the implementation**

Apply the first hunk of Step 1's `vocabulary.rs` diff (`pub struct DisplayName` and its `Debug`). Then the whole of `address.rs`, implementation and tests:

Create `backend/crates/domain/src/directory/address.rs`:

```rust
//! Turning a selection of addresses into a `mailto:` link or a text to copy (groups design
//! §4.3). Pure: the caller has already authorized the selection, decrypted the contact
//! addresses and written the audit entry (`fau_persistence::membership::export_addresses`).
//!
//! - A person selected through two groups appears once, and so does an address two people
//!   share: [`Recipients::new`] removes repeats, keeping first-seen order.
//! - More than [`BCC_ABOVE`] recipients default to Bcc, so a large mailing does not hand
//!   every recipient everyone else's address.
//! - Mail clients (notably on Windows) truncate `mailto:` URLs at about 2,000 characters,
//!   so a link longer than [`MAILTO_MAX_CHARS`] is refused and the screen points to copying.
//! - Every address is written as an RFC 5322 `addr-spec`, with the local part quoted when it
//!   is not a dot-atom, so a `,` or `;` inside an address cannot split it into two.

use crate::email::Email;

/// More recipients than this default to Bcc (§4.3).
pub const BCC_ABOVE: usize = 10;

/// The longest `mailto:` URL the screen will open (§4.3). One more character switches the
/// screen to copying.
pub const MAILTO_MAX_CHARS: usize = 1800;

/// Which header of the new message the addresses go into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientField {
    To,
    Bcc,
}

impl RecipientField {
    /// Bcc above [`BCC_ABOVE`] recipients, To otherwise. The screen's toggle starts here.
    pub fn default_for(count: usize) -> Self {
        if count > BCC_ABOVE {
            RecipientField::Bcc
        } else {
            RecipientField::To
        }
    }
}

/// What "Kopier adresser" puts between addresses. `, ` is the RFC 6068 and RFC 5322 list
/// separator; `; ` is what Outlook desktop expects if the implementation-time check on the
/// screen card finds it needs one (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopySeparator {
    Comma,
    Semicolon,
}

impl CopySeparator {
    pub fn as_str(self) -> &'static str {
        match self {
            CopySeparator::Comma => ", ",
            CopySeparator::Semicolon => "; ",
        }
    }
}

/// A `mailto:` link, or the reason there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mailto {
    Link(String),
    /// The URL would be `length` characters, more than [`MAILTO_MAX_CHARS`]. "Skriv e-post"
    /// is disabled, and a short explanation points to "Kopier adresser".
    TooLong {
        length: usize,
    },
}

/// The addresses of one selection, each once, in first-seen order.
#[derive(Clone, PartialEq, Eq)]
pub struct Recipients(Vec<Email>);

impl std::fmt::Debug for Recipients {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Recipients({} addresses)", self.0.len())
    }
}

impl Recipients {
    /// Removes repeats. `Email` is already trimmed and lower-cased, so equal addresses are
    /// equal strings.
    pub fn new(addresses: impl IntoIterator<Item = Email>) -> Self {
        let mut seen = std::collections::HashSet::new();
        Self(
            addresses
                .into_iter()
                .filter(|a| seen.insert(a.as_str().to_owned()))
                .collect(),
        )
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn default_field(&self) -> RecipientField {
        RecipientField::default_for(self.len())
    }

    /// `mailto:a@x.no,b@x.no` or `mailto:?bcc=a@x.no,b@x.no` (RFC 6068 §2: addresses are
    /// joined with a bare `,`; a space would have to be encoded and buys nothing).
    pub fn mailto(&self, field: RecipientField) -> Mailto {
        let joined = self
            .0
            .iter()
            .map(|a| percent_encode(&address_spec(a)))
            .collect::<Vec<_>>()
            .join(",");
        let url = match field {
            RecipientField::To => format!("mailto:{joined}"),
            RecipientField::Bcc => format!("mailto:?bcc={joined}"),
        };
        if url.len() > MAILTO_MAX_CHARS {
            Mailto::TooLong { length: url.len() }
        } else {
            Mailto::Link(url)
        }
    }

    /// The text "Kopier adresser" places on the clipboard, or in the fallback text box.
    pub fn copy_text(&self, separator: CopySeparator) -> String {
        self.0
            .iter()
            .map(address_spec)
            .collect::<Vec<_>>()
            .join(separator.as_str())
    }
}

/// RFC 5322 `atext`, plus any non-ASCII character (RFC 6531's `UTF8-non-ascii`).
fn is_atext(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c) || !c.is_ascii()
}

fn is_dot_atom(s: &str) -> bool {
    !s.is_empty()
        && s.split('.')
            .all(|part| !part.is_empty() && part.chars().all(is_atext))
}

/// The address as an RFC 5322 `addr-spec`: unchanged when its local part is a dot-atom,
/// otherwise with the local part quoted and `"` and `\` escaped.
pub fn address_spec(address: &Email) -> String {
    let (local, domain) = address
        .as_str()
        .rsplit_once('@')
        .expect("Email::parse guarantees an @");
    if is_dot_atom(local) {
        return address.as_str().to_owned();
    }
    let mut out = String::with_capacity(local.len() + domain.len() + 3);
    out.push('"');
    for c in local.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push_str("\"@");
    out.push_str(domain);
    out
}

/// Percent-encodes every byte except RFC 3986's unreserved characters and `@` (RFC 6068 §2:
/// encoding is always allowed, and `%`, `,`, `?`, `&`, `#`, `/` and non-ASCII must be).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(s: &str) -> Email {
        Email::parse(s).unwrap()
    }

    fn many(n: usize) -> Recipients {
        Recipients::new((0..n).map(|i| e(&format!("forelder{i}@example.no"))))
    }

    #[test]
    fn repeats_are_removed_in_first_seen_order() {
        let r = Recipients::new([
            e("kari@example.no"),
            e("ola@example.no"),
            e("KARI@example.no"),
            e("kari@example.no"),
        ]);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r.copy_text(CopySeparator::Comma),
            "kari@example.no, ola@example.no"
        );
    }

    #[test]
    fn bcc_is_the_default_above_ten() {
        assert_eq!(RecipientField::default_for(0), RecipientField::To);
        assert_eq!(RecipientField::default_for(10), RecipientField::To);
        assert_eq!(RecipientField::default_for(11), RecipientField::Bcc);
        assert_eq!(many(10).default_field(), RecipientField::To);
        assert_eq!(many(11).default_field(), RecipientField::Bcc);
        // Counted after repeats are removed: eleven selections of ten addresses stay To.
        let repeated =
            Recipients::new((0..11).map(|i| e(&format!("forelder{}@example.no", i % 10))));
        assert_eq!(repeated.default_field(), RecipientField::To);
    }

    #[test]
    fn to_and_bcc_links_join_with_a_bare_comma() {
        let r = Recipients::new([e("kari@example.no"), e("ola@example.no")]);
        assert_eq!(
            r.mailto(RecipientField::To),
            Mailto::Link("mailto:kari@example.no,ola@example.no".into())
        );
        assert_eq!(
            r.mailto(RecipientField::Bcc),
            Mailto::Link("mailto:?bcc=kari@example.no,ola@example.no".into())
        );
    }

    /// An address of exactly `n` characters, unique per `i`.
    fn address_of_len(i: usize, n: usize) -> String {
        let (tag, domain) = (format!("{i:04}"), "@example.no");
        format!("{tag}{}{domain}", "a".repeat(n - tag.len() - domain.len()))
    }

    /// Addresses whose `field` link is exactly `target` characters: 50-character addresses,
    /// then one that takes up the remainder.
    fn exactly(field: RecipientField, target: usize) -> Recipients {
        let mut len = match field {
            RecipientField::To => "mailto:".len(),
            RecipientField::Bcc => "mailto:?bcc=".len(),
        };
        let mut addresses = Vec::new();
        loop {
            let comma = usize::from(!addresses.is_empty());
            let left = target - len - comma;
            if left <= 100 {
                addresses.push(address_of_len(addresses.len(), left));
                break;
            }
            addresses.push(address_of_len(addresses.len(), 50));
            len += comma + 50;
        }
        Recipients::new(addresses.iter().map(|a| e(a)))
    }

    #[test]
    fn the_length_guard_switches_to_copying_above_1800_characters() {
        for field in [RecipientField::To, RecipientField::Bcc] {
            match exactly(field, MAILTO_MAX_CHARS).mailto(field) {
                Mailto::Link(url) => assert_eq!(url.len(), MAILTO_MAX_CHARS, "{field:?}"),
                other => panic!("{field:?}: {other:?}"),
            }
            assert_eq!(
                exactly(field, MAILTO_MAX_CHARS + 1).mailto(field),
                Mailto::TooLong {
                    length: MAILTO_MAX_CHARS + 1
                },
                "{field:?}"
            );
        }
    }

    #[test]
    fn the_guard_counts_encoded_characters() {
        // 'ø' is two UTF-8 bytes and six characters once percent-encoded.
        let r = Recipients::new([e("øystein@example.no")]);
        assert_eq!(
            r.mailto(RecipientField::To),
            Mailto::Link("mailto:%C3%B8ystein@example.no".into())
        );
    }

    #[test]
    fn addresses_that_are_not_dot_atoms_are_quoted_and_encoded() {
        let r = Recipients::new([
            e("kari+fau@example.no"),
            e("a,b@example.no"),
            e("per;paal@example.no"),
            e("q\"uote@example.no"),
            e(".dot@example.no"),
        ]);
        assert_eq!(
            r.copy_text(CopySeparator::Comma),
            "kari+fau@example.no, \"a,b\"@example.no, \"per;paal\"@example.no, \
             \"q\\\"uote\"@example.no, \".dot\"@example.no"
        );
        assert_eq!(
            r.copy_text(CopySeparator::Semicolon),
            "kari+fau@example.no; \"a,b\"@example.no; \"per;paal\"@example.no; \
             \"q\\\"uote\"@example.no; \".dot\"@example.no"
        );
        assert_eq!(
            r.mailto(RecipientField::To),
            Mailto::Link(
                "mailto:kari%2Bfau@example.no,%22a%2Cb%22@example.no,%22per%3Bpaal%22@example.no,\
                 %22q%5C%22uote%22@example.no,%22.dot%22@example.no"
                    .into()
            )
        );
    }

    #[test]
    fn url_delimiters_inside_an_address_are_encoded() {
        let r = Recipients::new([e("a?cc=x&b#c/d%e@example.no")]);
        assert_eq!(
            r.mailto(RecipientField::Bcc),
            Mailto::Link("mailto:?bcc=a%3Fcc%3Dx%26b%23c%2Fd%25e@example.no".into())
        );
    }

    #[test]
    fn debug_prints_the_count_only() {
        let r = Recipients::new([e("kari@example.no")]);
        assert_eq!(format!("{r:?}"), "Recipients(1 addresses)");
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-domain --lib directory::address membership::vocabulary`
Expected: PASS, 8 `directory::address` tests and the `vocabulary` tests. The boundary test builds links of exactly 1,800 characters (a `Link`) and 1,801 (`TooLong { length: 1801 }`) for both To and Bcc.

Mutation check: change `>` to `>=` in `mailto`, or `count > BCC_ABOVE` to `>=`, and a test fails.

- [ ] **Step 5: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/domain/src/membership/vocabulary.rs \
        backend/crates/domain/src/lib.rs \
        backend/crates/domain/src/directory/mod.rs \
        backend/crates/domain/src/directory/address.rs
git commit -m "Build mailto links and copy text for the member directory (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Locale-aware collation and the ordered listing

**Files:**
- Modify: `backend/Cargo.toml`, `backend/crates/domain/Cargo.toml` (and `backend/Cargo.lock` through cargo)
- Modify: `backend/crates/domain/src/directory/mod.rs`
- Create: `backend/crates/domain/src/directory/collation.rs`
- Create: `backend/crates/domain/src/directory/listing.rs`

**Interfaces:**
- Consumes: `DisplayName` (Task 2); `Email`.
- Produces:
  - in `fau_domain::directory::collation`:
    - `DEFAULT_LOCALE = "nb-NO"`;
    - `NameCollator::for_locale(tag: &str) -> NameCollator` (an unparsable tag gives `nb-NO`);
    - `NameCollator::compare(&self, &str, &str) -> Ordering`.
  - in `fau_domain::directory::listing`:
    - `enum SectionId { Fau, Group(Uuid) }` (`Copy`, `Hash`);
    - `struct Section { id: SectionId, title: Option<String>, members: Vec<Uuid> }`;
    - `enum ShownName { Named(DisplayName), Unnamed }`;
    - `enum Place { Unit(String), Cohort(String) }`;
    - `struct RoleHeld { name: String, place: Option<Place> }`;
    - `struct Person { membership_id: Uuid, name: ShownName, address: Email, is_guest: bool, roles: Vec<RoleHeld>, groups: Vec<Uuid> }`;
    - `struct Listing { sections: Vec<Section>, people: Vec<Person> }`;
    - `arrange(Vec<Section>, Vec<Person>, &NameCollator) -> Listing`.

- [ ] **Step 1: Add the dependencies**

Apply to `backend/Cargo.toml`:

```diff
--- a/backend/Cargo.toml
+++ b/backend/Cargo.toml
@@ -15,6 +15,8 @@
 clap = { version = "4", features = ["derive"] }
 flate2 = "1"
 getrandom = "0.4"
+icu_collator = "2.3"
+icu_locale_core = "2.3"
 jiff = { version = "0.2", default-features = false, features = ["std", "tzdb-bundle-always"] }
 reqwest = { version = "0.13", default-features = false, features = ["rustls", "query"] }
 serde = { version = "1", features = ["derive"] }
```

Apply to `backend/crates/domain/Cargo.toml`:

```diff
--- a/backend/crates/domain/Cargo.toml
+++ b/backend/crates/domain/Cargo.toml
@@ -9,9 +9,17 @@
 publish = false
 
 [dependencies]
+# Locale-aware ordering of decrypted names in the member directory (#3502, #3439): ICU4X,
+# the Unicode Consortium's library, with CLDR's per-locale collation data compiled in.
+# icu_locale_core and most of ICU4X's support crates are already in Cargo.lock through
+# `url`'s IDNA support.
+icu_collator = { workspace = true }
+icu_locale_core = { workspace = true }
 jiff = { workspace = true }
 serde = { workspace = true }
 unicode-normalization = { workspace = true }
+# Membership and group ids in the member directory's listing (#3502).
+uuid = { workspace = true }
 
 [features]
 # The sync planner's test builders and in-memory applier (`register::sync::testkit`), for
```

Run: `cargo build -p fau-domain`
Expected: `Cargo.lock` gains exactly `icu_collator`, `icu_collator_data`, `icu_locale_fallback`, `icu_locale_fallback_data`, `utf16_iter` and `write16`. Check with `git diff backend/Cargo.lock | grep '^+name'`. The other `icu_*` 2.3 crates are already locked through `url`.

Apply to `backend/crates/domain/src/directory/mod.rs`:

```diff
--- a/backend/crates/domain/src/directory/mod.rs
+++ b/backend/crates/domain/src/directory/mod.rs
@@ -7,3 +7,5 @@
 //! locale and builds the link or the text.
 
 pub mod address;
+pub mod collation;
+pub mod listing;
```

- [ ] **Step 2: Write the failing tests**

Create both files with their `#[cfg(test)] mod tests` blocks, as below, and empty implementations.

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p fau-domain --lib directory::collation directory::listing`
Expected: compile errors, `cannot find type NameCollator` and `cannot find function arrange`.

- [ ] **Step 4: Write the implementation**

Create `backend/crates/domain/src/directory/collation.rs`:

```rust
//! Locale-aware ordering of names (groups design §4.3, #3439). Names are encrypted, so the
//! database cannot sort them: the backend sorts after decrypting, inside the session.
//!
//! Byte order is wrong even for Bokmål (it puts Å before Æ and Ø), and #3439 rules out any
//! assumption of Latin collation or of two locales. So ordering is the Unicode Collation
//! Algorithm with CLDR's per-locale tailoring, through ICU4X (`icu_collator`), the Unicode
//! Consortium's own library. It is already in the dependency tree through `url`'s IDNA
//! support, so it adds one crate, not an ecosystem.
//!
//! **Data gap, recorded rather than hidden:** ICU4X's compiled data carries CLDR's tailoring
//! for `nb` and `nn` but not for the Sámi languages (`se`, `sma`, `smj`), which fall back to
//! the root order. Adding a Sámi locale therefore means generating ICU4X data that includes
//! it; the constructor below is the one place that changes.

use std::cmp::Ordering;
use std::fmt;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};
use icu_locale_core::{locale, Locale};

/// The default and fallback locale (#3439).
pub const DEFAULT_LOCALE: &str = "nb-NO";

pub struct NameCollator {
    collator: CollatorBorrowed<'static>,
}

impl fmt::Debug for NameCollator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NameCollator")
    }
}

impl NameCollator {
    /// A collator for a BCP 47 tag such as `nb-NO`, `nn-NO` or `en`. A tag that does not
    /// parse falls back to [`DEFAULT_LOCALE`]; a well-formed tag without its own data falls
    /// back along CLDR's chain to the root order.
    pub fn for_locale(tag: &str) -> Self {
        let locale: Locale = tag.parse().unwrap_or(locale!("nb-NO"));
        let collator = Collator::try_new(locale.into(), CollatorOptions::default())
            .or_else(|_| Collator::try_new(locale!("nb-NO").into(), CollatorOptions::default()))
            .expect("ICU4X compiled data covers nb-NO");
        Self { collator }
    }

    pub fn compare(&self, a: &str, b: &str) -> Ordering {
        self.collator.compare(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(tag: &str, names: &[&'static str]) -> Vec<&'static str> {
        let c = NameCollator::for_locale(tag);
        let mut v = names.to_vec();
        v.sort_by(|a, b| c.compare(a, b));
        v
    }

    const NAMES: [&str; 6] = ["Åse", "Øvre", "Ærlig", "Zakariassen", "Berit", "Anders"];

    #[test]
    fn bokmaal_puts_ae_oe_aa_after_z() {
        assert_eq!(
            sorted("nb-NO", &NAMES),
            ["Anders", "Berit", "Zakariassen", "Ærlig", "Øvre", "Åse"]
        );
        assert_eq!(sorted("nn-NO", &NAMES), sorted("nb-NO", &NAMES));
    }

    #[test]
    fn it_is_not_byte_order() {
        let mut bytes = NAMES.to_vec();
        bytes.sort_unstable();
        assert_eq!(
            bytes,
            ["Anders", "Berit", "Zakariassen", "Åse", "Ærlig", "Øvre"]
        );
        assert_ne!(sorted("nb-NO", &NAMES), bytes);
    }

    #[test]
    fn another_locale_orders_differently() {
        // English folds the letters into A and O: a third order, from the same names.
        assert_eq!(
            sorted("en", &NAMES),
            ["Ærlig", "Anders", "Åse", "Berit", "Øvre", "Zakariassen"]
        );
    }

    #[test]
    fn case_does_not_split_a_name_from_its_neighbours() {
        assert_eq!(
            sorted("nb-NO", &["bjørn", "Anne", "Bjørg", "anders"]),
            ["anders", "Anne", "Bjørg", "bjørn"]
        );
    }

    #[test]
    fn an_unparsable_tag_falls_back_to_bokmaal() {
        assert_eq!(sorted("not a tag!", &NAMES), sorted("nb-NO", &NAMES));
        assert_eq!(sorted("", &NAMES), sorted("nb-NO", &NAMES));
    }
}
```

Create `backend/crates/domain/src/directory/listing.rs`:

```rust
//! The directory as the screen shows it (groups design §4.3), after persistence has
//! authorized and read it and the session has decrypted names and addresses.
//!
//! [`arrange`] puts it in order: the FAU-wide section first, then groups by name; people in
//! every section by name; each person's roles by name. Every comparison of names is the
//! viewer's locale's collation ([`NameCollator`]), never byte order, and ties fall back to
//! the id so the order is stable. A person listed in two sections is one [`Person`].

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::collation::NameCollator;
use crate::email::Email;
use crate::membership::vocabulary::DisplayName;

/// One heading of the directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionId {
    /// Every current member and admin: "Hele FAU". Never shown to a guest, and never lists
    /// one (§3.2: an FAU-wide audience never includes guests).
    Fau,
    Group(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: SectionId,
    /// The decrypted group name; `None` for [`SectionId::Fau`], whose heading the screen
    /// takes from the catalogue.
    pub title: Option<String>,
    pub members: Vec<Uuid>,
}

/// A name as the directory shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShownName {
    Named(DisplayName),
    /// A membership created before migration 0008, which has no name. Sorted last. Bokmål
    /// source string for the catalogue (#3439): "Navn ikke oppgitt".
    Unnamed,
}

/// Where a role sits in the school structure, when it sits anywhere (plaintext, ADR-003
/// decision 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Unit(String),
    Cohort(String),
}

/// One role held today, as "what a person represents" shows it: the role's name, plus the
/// unit or cohort it sits on (§4.1). Past roles are never listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleHeld {
    pub name: String,
    pub place: Option<Place>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub membership_id: Uuid,
    pub name: ShownName,
    /// The contact address, or the login address when the member set none (D3).
    pub address: Email,
    /// Marked "Gjest" on the screen (Bokmål source string for the catalogue).
    pub is_guest: bool,
    pub roles: Vec<RoleHeld>,
    /// The listed groups this person is in, in section order.
    pub groups: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub sections: Vec<Section>,
    /// Every person listed in any section, once, in name order.
    pub people: Vec<Person>,
}

fn compare_names(c: &NameCollator, a: &ShownName, b: &ShownName) -> Ordering {
    match (a, b) {
        (ShownName::Named(a), ShownName::Named(b)) => c.compare(a.as_str(), b.as_str()),
        (ShownName::Named(_), ShownName::Unnamed) => Ordering::Less,
        (ShownName::Unnamed, ShownName::Named(_)) => Ordering::Greater,
        (ShownName::Unnamed, ShownName::Unnamed) => Ordering::Equal,
    }
}

fn place_text(p: &Option<Place>) -> &str {
    match p {
        None => "",
        Some(Place::Unit(s) | Place::Cohort(s)) => s,
    }
}

/// Orders the directory for one viewer's locale. Repeated people and repeated section
/// members are dropped, keeping the first; a section member with no [`Person`] is dropped
/// too, so a section can never list someone the people list does not describe.
pub fn arrange(sections: Vec<Section>, people: Vec<Person>, collator: &NameCollator) -> Listing {
    let mut seen = HashSet::new();
    let mut people: Vec<Person> = people
        .into_iter()
        .filter(|p| seen.insert(p.membership_id))
        .collect();
    people.sort_by(|a, b| {
        compare_names(collator, &a.name, &b.name).then(a.membership_id.cmp(&b.membership_id))
    });
    let rank: HashMap<Uuid, usize> = people
        .iter()
        .enumerate()
        .map(|(i, p)| (p.membership_id, i))
        .collect();

    let mut sections = sections;
    sections.sort_by(|a, b| match (a.id, b.id) {
        (SectionId::Fau, SectionId::Fau) => Ordering::Equal,
        (SectionId::Fau, SectionId::Group(_)) => Ordering::Less,
        (SectionId::Group(_), SectionId::Fau) => Ordering::Greater,
        (SectionId::Group(x), SectionId::Group(y)) => collator
            .compare(
                a.title.as_deref().unwrap_or(""),
                b.title.as_deref().unwrap_or(""),
            )
            .then(x.cmp(&y)),
    });
    for s in &mut sections {
        let mut in_section = HashSet::new();
        s.members
            .retain(|m| rank.contains_key(m) && in_section.insert(*m));
        s.members.sort_by_key(|m| rank[m]);
    }
    let group_rank: HashMap<Uuid, usize> = sections
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s.id {
            SectionId::Group(g) => Some((g, i)),
            SectionId::Fau => None,
        })
        .collect();

    for p in &mut people {
        p.roles.sort_by(|a, b| {
            collator
                .compare(&a.name, &b.name)
                .then_with(|| collator.compare(place_text(&a.place), place_text(&b.place)))
        });
        let mut in_person = HashSet::new();
        p.groups
            .retain(|g| group_rank.contains_key(g) && in_person.insert(*g));
        p.groups.sort_by_key(|g| group_rank[g]);
    }
    Listing { sections, people }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn person(n: u128, name: Option<&str>, groups: &[u128]) -> Person {
        Person {
            membership_id: id(n),
            name: match name {
                Some(s) => ShownName::Named(DisplayName::parse(s).unwrap()),
                None => ShownName::Unnamed,
            },
            address: Email::parse(&format!("p{n}@example.no")).unwrap(),
            is_guest: false,
            roles: Vec::new(),
            groups: groups.iter().map(|g| id(*g)).collect(),
        }
    }

    fn section(sid: SectionId, title: Option<&str>, members: &[u128]) -> Section {
        Section {
            id: sid,
            title: title.map(str::to_owned),
            members: members.iter().map(|m| id(*m)).collect(),
        }
    }

    fn names(l: &Listing, s: usize) -> Vec<String> {
        l.sections[s]
            .members
            .iter()
            .map(|m| {
                match &l
                    .people
                    .iter()
                    .find(|p| p.membership_id == *m)
                    .unwrap()
                    .name
                {
                    ShownName::Named(n) => n.as_str().to_owned(),
                    ShownName::Unnamed => "-".to_owned(),
                }
            })
            .collect()
    }

    #[test]
    fn a_person_in_two_groups_is_one_person_listed_in_both() {
        let people = vec![
            person(1, Some("Åse"), &[100, 101]),
            person(2, Some("Berit"), &[100]),
            person(1, Some("Åse"), &[100, 101]),
        ];
        let sections = vec![
            section(SectionId::Group(id(101)), Some("Styret"), &[1]),
            section(SectionId::Group(id(100)), Some("Dugnad"), &[1, 2, 1]),
        ];
        let l = arrange(sections, people, &NameCollator::for_locale("nb-NO"));
        assert_eq!(l.people.len(), 2);
        assert_eq!(names(&l, 0), ["Berit", "Åse"], "Dugnad sorts before Styret");
        assert_eq!(names(&l, 1), ["Åse"]);
        let aase = l.people.iter().find(|p| p.membership_id == id(1)).unwrap();
        assert_eq!(aase.groups, [id(100), id(101)], "in section order");
    }

    #[test]
    fn names_follow_the_viewers_locale_and_unnamed_sort_last() {
        let people = vec![
            person(1, Some("Åse"), &[]),
            person(2, None, &[]),
            person(3, Some("Øvre"), &[]),
            person(4, Some("Anders"), &[]),
        ];
        let sections = vec![section(SectionId::Fau, None, &[1, 2, 3, 4])];
        let nb = arrange(
            sections.clone(),
            people.clone(),
            &NameCollator::for_locale("nb-NO"),
        );
        assert_eq!(names(&nb, 0), ["Anders", "Øvre", "Åse", "-"]);
        let en = arrange(sections, people, &NameCollator::for_locale("en"));
        assert_eq!(names(&en, 0), ["Anders", "Åse", "Øvre", "-"]);
    }

    #[test]
    fn the_fau_section_comes_first_and_groups_follow_by_name() {
        let people = vec![person(1, Some("Kari"), &[])];
        let sections = vec![
            section(SectionId::Group(id(10)), Some("Årsmøtekomiteen"), &[1]),
            section(SectionId::Group(id(11)), Some("Øvingsgruppa"), &[1]),
            section(SectionId::Fau, None, &[1]),
            section(SectionId::Group(id(12)), Some("Dugnad"), &[1]),
        ];
        let l = arrange(sections, people, &NameCollator::for_locale("nb-NO"));
        let order: Vec<SectionId> = l.sections.iter().map(|s| s.id).collect();
        assert_eq!(
            order,
            [
                SectionId::Fau,
                SectionId::Group(id(12)),
                SectionId::Group(id(11)),
                SectionId::Group(id(10)),
            ]
        );
    }

    #[test]
    fn a_section_never_lists_someone_the_people_list_does_not_describe() {
        let l = arrange(
            vec![section(SectionId::Fau, None, &[1, 9])],
            vec![person(1, Some("Kari"), &[77])],
            &NameCollator::for_locale("nb-NO"),
        );
        assert_eq!(l.sections[0].members, [id(1)]);
        assert!(l.people[0].groups.is_empty(), "no section for group 77");
    }

    #[test]
    fn roles_are_ordered_by_name_then_place() {
        let mut p = person(1, Some("Kari"), &[]);
        p.roles = vec![
            RoleHeld {
                name: "Kontaktforelder".into(),
                place: Some(Place::Unit("7B".into())),
            },
            RoleHeld {
                name: "Kasserer".into(),
                place: None,
            },
            RoleHeld {
                name: "Kontaktforelder".into(),
                place: Some(Place::Unit("7A".into())),
            },
        ];
        let l = arrange(Vec::new(), vec![p], &NameCollator::for_locale("nb-NO"));
        let shown: Vec<(&str, &str)> = l.people[0]
            .roles
            .iter()
            .map(|r| (r.name.as_str(), place_text(&r.place)))
            .collect();
        assert_eq!(
            shown,
            [
                ("Kasserer", ""),
                ("Kontaktforelder", "7A"),
                ("Kontaktforelder", "7B")
            ]
        );
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p fau-domain --lib directory`
Expected: PASS, 18 tests in `directory` (8 address, 5 collation, 5 listing). `bokmaal_puts_ae_oe_aa_after_z`, `it_is_not_byte_order` and `another_locale_orders_differently` show three different orders from one list of names.

Mutation check: replace `self.collator.compare(a, b)` with `a.cmp(b)`, and three tests fail.

- [ ] **Step 6: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 7: Commit**

```bash
git add backend/Cargo.toml \
        backend/Cargo.lock \
        backend/crates/domain/Cargo.toml \
        backend/crates/domain/src/directory/mod.rs \
        backend/crates/domain/src/directory/collation.rs \
        backend/crates/domain/src/directory/listing.rs
git commit -m "Order the directory with locale-aware collation through ICU4X (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3b: Amend migration 0008 for D3: a revoked membership holds no name

Added 28 September 2026 for Erik's decision **D3**, after Tasks 1–3 were built. A display name is valid only while the membership is active; once it ends, the name is cleared like the contact address, and history shows role and year instead (Rulings (D3) D3-1 to D3-8). Task 3b keeps its letter so that Tasks 4–9 and the execution ledger's rulings Q1–Q8 keep their numbers.

`0008` is edited **in place**, which the Global Constraints allow only while no persistent database has applied it. Step 1 checks that first.

**Files:**
- Modify: `backend/migrations/0008_member_directory.sql`
- Modify: `backend/crates/app/tests/directory_schema.rs`

**Interfaces:**
- Consumes: Task 1's migration and tests.
- Produces:
  - check `memberships_display_name_only_while_current`: `revoked_at is null or encrypted_display_name is null`, the twin of `memberships_contact_email_only_while_current`;
  - `name_erased_at`, unchanged in type, now documented as the Article 17 marker that also suppresses the role-and-year label (Ruling D3-4);
  - `schema_contract` stays at version 8.

- [ ] **Step 1: Confirm that no persistent database has applied 0008 (read only)**

The persistent database is `fau` in the compose `db`, the one the `migrate` service targets. The `fau_test_*` databases are excluded on purpose: the test template's name is a hash of the migration set (`common::TEMPLATE_PREFIX`), so an edited `0008` builds a fresh template, and the per-test databases are throwaway.

```bash
for d in $(psql -Atq postgres://postgres:postgres@db:5432/postgres \
             -c "select datname from pg_database where datallowconn and datname not like 'fau\_test\_%' order by 1"); do
  printf '%s: ' "$d"
  psql -Atq "postgres://postgres:postgres@db:5432/$d" -c \
    "select coalesce(max(n.nspname || '._sqlx_migrations'), 'no migrations table')
       from pg_class c join pg_namespace n on n.oid = c.relnamespace
      where c.relname = '_sqlx_migrations'"
done
```

Expected (as observed on 28 September 2026):
```
fau: no migrations table
postgres: no migrations table
template1: no migrations table
```

If any database prints a table name, run `select version from <that table> where version = 8` there. If that returns a row, **stop**: do not edit `0008`. Report to the controller, because the change then becomes a new `0009`.

- [ ] **Step 2: Write the failing test**

Apply to `backend/crates/app/tests/directory_schema.rs`:

```diff
--- a/backend/crates/app/tests/directory_schema.rs
+++ b/backend/crates/app/tests/directory_schema.rs
@@ -1,5 +1,7 @@
 //! Migration 0008: the member directory's fields on `memberships` (groups design §4.1,
-//! §4.2), proven in SQL before any Rust depends on them.
+//! §4.2), proven in SQL before any Rust depends on them. Under Erik's D3 (28 September
+//! 2026) neither field outlives the membership: a revoked membership holds no name and no
+//! contact address.
 
 mod common;
 use common::TestDb;
@@ -98,52 +100,72 @@ async fn both_fields_hold_only_a_bounded_envelope() {
 }
 
 #[tokio::test]
-async fn a_revoked_membership_holds_no_contact_email() {
+async fn a_revoked_membership_holds_neither_a_name_nor_a_contact_email() {
     let db = TestDb::migrated().await;
     let pool = db.admin_pool();
     let (_, m) = seed(&pool).await;
-    set(&pool, m, "encrypted_contact_email", Some(envelope(1, 60)))
-        .await
-        .unwrap();
-    set(&pool, m, "encrypted_display_name", Some(envelope(1, 60)))
-        .await
-        .unwrap();
-
-    // Revoking without clearing the address is refused.
-    let err =
-        sqlx::query("update memberships set revoked_at = '2026-09-23T10:00:00Z' where id = $1")
+    let checks = [
+        (
+            "encrypted_display_name",
+            "memberships_display_name_only_while_current",
+        ),
+        (
+            "encrypted_contact_email",
+            "memberships_contact_email_only_while_current",
+        ),
+    ];
+    let revoke = |clear: &'static str| {
+        let pool = pool.clone();
+        async move {
+            sqlx::query(&format!(
+                "update memberships set revoked_at = '2026-09-23T10:00:00Z'{clear} where id = $1"
+            ))
             .bind(m)
             .execute(&pool)
             .await
-            .unwrap_err();
-    assert_eq!(
-        constraint_name(&err).as_deref(),
-        Some("memberships_contact_email_only_while_current")
-    );
+            .map(|_| ())
+        }
+    };
 
-    // Clearing it in the same statement is accepted, and the name stays (§4.2).
-    sqlx::query(
-        "update memberships set revoked_at = '2026-09-23T10:00:00Z', encrypted_contact_email = null
-          where id = $1",
-    )
-    .bind(m)
-    .execute(&pool)
-    .await
-    .unwrap();
-    let err = set(&pool, m, "encrypted_contact_email", Some(envelope(1, 60)))
+    // Each field on its own blocks the revocation, with its own check.
+    for (column, constraint) in checks {
+        set(&pool, m, column, Some(envelope(1, 60))).await.unwrap();
+        let err = revoke("").await.unwrap_err();
+        assert_eq!(
+            constraint_name(&err).as_deref(),
+            Some(constraint),
+            "{column}"
+        );
+        set(&pool, m, column, None).await.unwrap();
+    }
+
+    // Clearing both in the same statement is accepted: under D3 the name goes too.
+    for (column, _) in checks {
+        set(&pool, m, column, Some(envelope(1, 60))).await.unwrap();
+    }
+    revoke(", encrypted_display_name = null, encrypted_contact_email = null")
         .await
-        .unwrap_err();
-    assert_eq!(
-        constraint_name(&err).as_deref(),
-        Some("memberships_contact_email_only_while_current")
-    );
-    let name: Option<Vec<u8>> =
-        sqlx::query_scalar("select encrypted_display_name from memberships where id = $1")
-            .bind(m)
-            .fetch_one(&pool)
+        .unwrap();
+    for (column, constraint) in checks {
+        let err = set(&pool, m, column, Some(envelope(1, 60)))
             .await
-            .unwrap();
-    assert_eq!(name, Some(envelope(1, 60)));
+            .unwrap_err();
+        assert_eq!(
+            constraint_name(&err).as_deref(),
+            Some(constraint),
+            "{column}"
+        );
+    }
+
+    // Positive control, the re-invitation path: reopened, the row takes a new name.
+    sqlx::query("update memberships set revoked_at = null where id = $1")
+        .bind(m)
+        .execute(&pool)
+        .await
+        .unwrap();
+    set(&pool, m, "encrypted_display_name", Some(envelope(1, 60)))
+        .await
+        .unwrap();
 }
 
 #[tokio::test]
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p fau-app --test directory_schema`
Expected: `a_revoked_membership_holds_neither_a_name_nor_a_contact_email` FAILS. The revocation with only a name set is accepted, so `unwrap_err` panics with `called Result::unwrap_err() on an Ok value`. The other 3 tests pass.

- [ ] **Step 4: Amend the migration**

Overwrite `backend/migrations/0008_member_directory.sql` with:

```sql
-- 0008: the member directory's two fields (#3502; groups design §4.1, §4.2, as amended by
-- Erik's D3 of 28 September 2026).
--
-- Both are per membership, so one person can go by different names in two FAU-er, and
-- both are content: the backend encrypts them under the FAU's record key with AAD
-- (tenant, 'memberships', <column>, membership id). This table holds the envelopes only,
-- and no key material. The checks are structural, as for groups.encrypted_name in 0007:
-- version byte 1 and 42..512 octets, i.e. 41 bytes of version, nonce and tag around
-- 1..471 bytes of plaintext. A display name (1..100 characters, at most 400 bytes of UTF-8)
-- and an address (at most 254 bytes) both fit with headroom.
--
-- **Neither field outlives the membership (D3).** A membership has ended when it is
-- revoked, or when none of its role assignments is still running or yet to start. The
-- two *_only_while_current checks hold that for revocation; fau_persistence's
-- clear_ended_profiles sweep clears both fields from a membership whose roles simply ran
-- out. History then shows an ended membership as its role and year ("Leder 2025–2026"),
-- computed from role_assignments at read time, never a name. So there is no name history
-- to keep.
--
-- * encrypted_display_name: required by the code when an invitation is accepted or an FAU
--   activated, so every membership created from here on has one while it is active.
--   Nullable because rows created before this migration have none, because an ended
--   membership holds none, and because an Article 17 erasure removes it.
-- * encrypted_contact_email: optional, the member's own statement, never verified and never
--   mailed by the system.
-- * name_erased_at: an Article 17 erasure. History shows "Tidligere medlem" for the
--   membership, never a name and not even its role and year, and the row cannot be
--   accepted into again.
alter table memberships
  add column encrypted_display_name  bytea,
  add column encrypted_contact_email bytea,
  add column name_erased_at          timestamptz;

alter table memberships add constraint memberships_display_name_is_an_envelope
  check (encrypted_display_name is null
         or (get_byte(encrypted_display_name, 0) = 1
             and octet_length(encrypted_display_name) between 42 and 512));
alter table memberships add constraint memberships_contact_email_is_an_envelope
  check (encrypted_contact_email is null
         or (get_byte(encrypted_contact_email, 0) = 1
             and octet_length(encrypted_contact_email) between 42 and 512));
alter table memberships add constraint memberships_display_name_only_while_current
  check (revoked_at is null or encrypted_display_name is null);
alter table memberships add constraint memberships_contact_email_only_while_current
  check (revoked_at is null or encrypted_contact_email is null);
alter table memberships add constraint memberships_erasure_leaves_nothing
  check (name_erased_at is null
         or (encrypted_display_name is null and encrypted_contact_email is null));

-- fau_app already holds select, insert, update, delete on memberships (0002).

insert into schema_contract (version) values (8);
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test directory_schema --test migrations --test schema_review --test membership_schema`
Expected: PASS. `directory_schema` shows 4 passed. The others are unchanged: no code writes a name yet, so no existing test revokes a named membership.

Mutation check: drop the new `memberships_display_name_only_while_current`, and the test fails again as in Step 3.

- [ ] **Step 6: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 7: Commit**

```bash
git add backend/migrations/0008_member_directory.sql \
        backend/crates/app/tests/directory_schema.rs
git commit -m "Amend migration 0008 for D3: a revoked membership holds no name (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Capture the display name at acceptance and activation; revocation takes both fields

This changes #3418/#3501's accept path (spec §8's #3418 row). Every caller now passes a `MemberProfile`. Under D3, revocation clears the name as well as the contact address (Ruling D3-2).

**Files:**
- Modify: `backend/crates/persistence/src/membership/error.rs`, `sql.rs`, `invitations.rs`, `signup.rs`, `roles.rs`, `mod.rs`
- Create: `backend/crates/persistence/src/membership/profile.rs`
- Modify: `backend/crates/app/tests/common/membership.rs`
- Modify: `backend/crates/app/tests/{outbox,guests,invitations,requests,signup,handover_recovery,roles}.rs`
- Test: `backend/crates/app/tests/member_profile.rs`

**Interfaces:**
- Consumes:
  - the columns and checks from Tasks 1 and 3b;
  - `fau_crypto::Ciphertext` (`from_stored`, `as_bytes`, `len`);
  - `lock_tenant`, `upsert_verified_account` and `hash_token`/`looks_like_token`.
- Produces:
  - `DISPLAY_NAME_AAD = ("memberships", "encrypted_display_name")` and `CONTACT_EMAIL_AAD = ("memberships", "encrypted_contact_email")`;
  - `MEMBER_FIELD_CIPHERTEXT_BYTES = 42..=512`;
  - `struct MemberProfile { membership_id: Uuid, encrypted_display_name: Ciphertext, encrypted_contact_email: Option<Ciphertext> }`;
  - `AcceptInvitation { token, acceptor, admin_end_override, profile: MemberProfile }` and `Activation { tenant_id, registrant, profile: MemberProfile }`;
  - `prepare_acceptance(pool, token: &str, acceptor: &VerifiedEmail) -> Result<AcceptanceTarget, MembershipError>`, with `struct AcceptanceTarget { tenant_id: Uuid, membership_id: Uuid }`;
  - `MembershipError::{DisplayNameMalformed, ContactEmailMalformed, AcceptanceTargetChanged, MembershipErased, MembershipEnded}` (`MembershipEnded` is first returned in Task 5);
  - `revoke_membership` clears `encrypted_display_name` and `encrypted_contact_email` in the statement that revokes;
  - `pub(crate) ensure_membership(conn, tenant_id, account_id, expected_id) -> Result<(Uuid, bool), _>`;
  - `pub(crate) write_profile(conn, tenant_id, &MemberProfile)`;
  - `pub(crate) check_display_name` and `check_contact_email`;
  - test helpers `placeholder_envelope() -> Ciphertext`, `fresh_profile() -> MemberProfile` and `profile_for(pool, token, acceptor) -> MemberProfile`.

- [ ] **Step 1: Write the failing test**

Create `backend/crates/app/tests/member_profile.rs`:

```rust
//! The directory fields enter the system (groups design §4.1, §8's #3418 row; #3502): a
//! display name is required when an invitation is accepted or an FAU activated, a contact
//! address is optional, and both are bound to the membership `prepare_acceptance` names.
//! Under D3 both leave with the membership: revocation clears them.

mod common;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

/// An envelope whose bytes say which one it is, so a test can tell two apart.
fn envelope(marker: u8) -> Ciphertext {
    let mut v = vec![1u8, marker];
    v.extend_from_slice(&[0u8; 40]);
    Ciphertext::from_stored(v)
}

fn year() -> fau_domain::membership::period::Period {
    period(day(2026, 9, 1), day(2027, 9, 1))
}

async fn invite(pool: &PgPool, fau: &Fau, address: &str) -> String {
    issue_invitation(
        pool,
        IssueInvitation {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            recipient: email(address),
            roles: vec![OfferedRole {
                role: new_role("Medlem", CapabilityClass::Member),
                period: year(),
            }],
            handover_grant_id: None,
            message: None,
        },
        at(T0),
    )
    .await
    .unwrap()
    .token
    .expose()
    .to_owned()
}

async fn fields(pool: &PgPool, membership: Uuid) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    sqlx::query_as(
        "select encrypted_display_name, encrypted_contact_email from memberships where id = $1",
    )
    .bind(membership)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn pending_invitations(pool: &PgPool, recipient: &str) -> i64 {
    sqlx::query_scalar(
        "select count(*) from invitations where recipient_email = $1 and accepted_at is null",
    )
    .bind(recipient)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn acceptance_stores_the_name_and_the_optional_address_on_the_prepared_membership() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;

    for (address, contact) in [
        ("kari@example.test", Some(envelope(7))),
        ("ola@example.test", None),
    ] {
        let token = invite(&pool, &fau, address).await;
        let target = prepare_acceptance(&pool, &token, &verified(address))
            .await
            .unwrap();
        assert_eq!(target.tenant_id, fau.tenant_id);
        let accepted = accept_invitation(
            &pool,
            AcceptInvitation {
                token,
                acceptor: verified(address),
                admin_end_override: None,
                profile: MemberProfile {
                    membership_id: target.membership_id,
                    encrypted_display_name: envelope(3),
                    encrypted_contact_email: contact.clone(),
                },
            },
            at(T0),
        )
        .await
        .unwrap();
        assert_eq!(accepted.membership_id, target.membership_id, "{address}");
        let (name, stored_contact) = fields(&pool, accepted.membership_id).await;
        assert_eq!(name.as_deref(), Some(envelope(3).as_bytes()), "{address}");
        assert_eq!(
            stored_contact.as_deref(),
            contact.as_ref().map(|c| c.as_bytes()),
            "{address}"
        );
    }
}

#[tokio::test]
async fn activation_stores_the_registrants_name() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let school_id = school(&pool, "activation-name").await;
    let pending = create_pending_tenant(
        &pool,
        signup(school_id, "reg@example.test", "reg@example.test", t0),
        t0,
    )
    .await
    .unwrap();
    let chosen = Uuid::now_v7();
    let activated = activate_tenant(
        &pool,
        Activation {
            tenant_id: pending.tenant_id,
            registrant: verified("reg@example.test"),
            profile: MemberProfile {
                membership_id: chosen,
                encrypted_display_name: envelope(5),
                encrypted_contact_email: Some(envelope(6)),
            },
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(activated.membership_id, chosen);
    let (name, contact) = fields(&pool, chosen).await;
    assert_eq!(name.as_deref(), Some(envelope(5).as_bytes()));
    assert_eq!(contact.as_deref(), Some(envelope(6).as_bytes()));
}

#[tokio::test]
async fn a_malformed_field_is_refused_before_anything_is_written() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let token = invite(&pool, &fau, "kari@example.test").await;
    let short = Ciphertext::from_stored(vec![1u8; 41]);
    let wrong_version = Ciphertext::from_stored(vec![2u8; 60]);
    for (profile, want) in [
        (
            MemberProfile {
                encrypted_display_name: short.clone(),
                ..fresh_profile()
            },
            MembershipError::DisplayNameMalformed,
        ),
        (
            MemberProfile {
                encrypted_display_name: wrong_version.clone(),
                ..fresh_profile()
            },
            MembershipError::DisplayNameMalformed,
        ),
        (
            MemberProfile {
                encrypted_contact_email: Some(Ciphertext::from_stored(vec![1u8; 513])),
                ..fresh_profile()
            },
            MembershipError::ContactEmailMalformed,
        ),
    ] {
        let err = accept_invitation(
            &pool,
            AcceptInvitation {
                token: token.clone(),
                acceptor: verified("kari@example.test"),
                admin_end_override: None,
                profile,
            },
            at(T0),
        )
        .await
        .unwrap_err();
        assert_eq!(err, want);
        assert_eq!(pending_invitations(&pool, "kari@example.test").await, 1);
    }
    assert_eq!(
        count(
            &pool,
            "select count(*) from accounts where email = 'kari@example.test'"
        )
        .await,
        0,
        "no account was created"
    );
}

#[tokio::test]
async fn a_profile_for_another_membership_is_refused() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let first = add_member(
        &pool,
        &fau,
        "kari@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year(),
        t0,
    )
    .await;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: first.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        fields(&pool, first.membership_id).await,
        (None, None),
        "revocation took the name (D3)"
    );

    // Invited back: prepare names the old membership, which acceptance reopens.
    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
        .await
        .unwrap();
    assert_eq!(target.membership_id, first.membership_id);

    // A profile encrypted for any other id would be undecryptable on that row: refused,
    // and the membership stays revoked.
    let err = accept_invitation(
        &pool,
        AcceptInvitation {
            token: token.clone(),
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: fresh_profile(),
        },
        t0,
    )
    .await
    .unwrap_err();
    assert_eq!(err, MembershipError::AcceptanceTargetChanged);
    assert_eq!(pending_invitations(&pool, "kari@example.test").await, 1);
    let revoked: bool =
        sqlx::query_scalar("select revoked_at is not null from memberships where id = $1")
            .bind(first.membership_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked);

    // Positive control: the prepared id is accepted, and the reopened row is named again.
    accept_invitation(
        &pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(9),
                encrypted_contact_email: None,
            },
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        fields(&pool, first.membership_id).await.0.as_deref(),
        Some(envelope(9).as_bytes())
    );
}

#[tokio::test]
async fn prepare_names_only_this_faus_membership_and_only_for_the_recipient() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    // Kari is already a member of B; A invites her.
    let in_b = add_member(
        &pool,
        &b,
        "kari@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year(),
        t0,
    )
    .await;
    let token = invite(&pool, &a, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
        .await
        .unwrap();
    assert_eq!(target.tenant_id, a.tenant_id);
    assert_ne!(
        target.membership_id, in_b.membership_id,
        "a membership in another FAU is never reused"
    );
    let again = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
        .await
        .unwrap();
    assert_ne!(again.membership_id, target.membership_id, "fresh each time");

    // Every other failure looks the same.
    for (token, reader) in [
        (token.as_str(), "ola@example.test"),
        ("not-a-token", "kari@example.test"),
        (&"a".repeat(64)[..], "kari@example.test"),
    ] {
        assert_eq!(
            prepare_acceptance(&pool, token, &verified(reader))
                .await
                .unwrap_err(),
            MembershipError::UnknownInvitation,
            "{reader}"
        );
    }
}

#[tokio::test]
async fn prepare_answers_for_an_expired_invitation_so_acceptance_can_say_why() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let token = invite(&pool, &fau, "kari@example.test").await;
    let late = at("2027-06-01T10:00:00Z");
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
        .await
        .unwrap();
    let err = accept_invitation(
        &pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                ..fresh_profile()
            },
        },
        late,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, MembershipError::Acceptance(_)), "{err:?}");
}

/// Both fields go with the membership (D3). Without the clearing in `revoke_membership`,
/// migration 0008's `memberships_display_name_only_while_current` or
/// `memberships_contact_email_only_while_current` refuses the revocation outright.
#[tokio::test]
async fn revoking_a_membership_clears_its_name_and_contact_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let token = invite(&pool, &fau, "kari@example.test").await;
    let target = prepare_acceptance(&pool, &token, &verified("kari@example.test"))
        .await
        .unwrap();
    let kari = accept_invitation(
        &pool,
        AcceptInvitation {
            token,
            acceptor: verified("kari@example.test"),
            admin_end_override: None,
            profile: MemberProfile {
                membership_id: target.membership_id,
                encrypted_display_name: envelope(3),
                encrypted_contact_email: Some(envelope(4)),
            },
        },
        t0,
    )
    .await
    .unwrap()
    .membership_id;
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: kari,
            membership_id: kari,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    assert_eq!(fields(&pool, kari).await, (None, None));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p fau-app --test member_profile`
Expected: compile errors, `cannot find function prepare_acceptance`, `cannot find struct MemberProfile` and `struct AcceptInvitation has no field named profile`.

- [ ] **Step 3: Add the error variants**

Apply to `backend/crates/persistence/src/membership/error.rs`:

```diff
--- a/backend/crates/persistence/src/membership/error.rs
+++ b/backend/crates/persistence/src/membership/error.rs
@@ -128,6 +128,22 @@ pub enum MembershipError {
     #[error("cohort not found")]
     UnknownCohort,
 
+    // The member directory (#3502).
+    #[error("the display name is not a well-formed ciphertext")]
+    DisplayNameMalformed,
+    #[error("the contact address is not a well-formed ciphertext")]
+    ContactEmailMalformed,
+    /// The membership the profile was encrypted for is not the one acceptance found: another
+    /// acceptance created it in between. `prepare_acceptance` again and re-encrypt.
+    #[error("the membership changed since the acceptance was prepared")]
+    AcceptanceTargetChanged,
+    #[error("the membership's name was erased")]
+    MembershipErased,
+    /// The membership has ended -- it is revoked, or none of its role assignments is still
+    /// running or yet to start -- so it holds no name (Erik's D3, 28 September 2026).
+    #[error("the membership has ended")]
+    MembershipEnded,
+
     // Infrastructure.
     #[error("the operating system's random source failed")]
     Randomness,
```

- [ ] **Step 4: Bind `ensure_membership` to the expected id**

Apply to `backend/crates/persistence/src/membership/sql.rs`:

```diff
--- a/backend/crates/persistence/src/membership/sql.rs
+++ b/backend/crates/persistence/src/membership/sql.rs
@@ -248,22 +248,36 @@
     .await?)
 }
 
-/// Finds or creates the account's membership in the tenant. A revoked membership is
-/// reopened rather than duplicated (one membership per account per FAU, #3412); the
-/// revocation stays in the audit log. Returns the id and whether it already existed.
+/// Finds or creates the account's membership in the tenant, as `expected_id`. A revoked
+/// membership is reopened rather than duplicated (one membership per account per FAU,
+/// #3412); the revocation stays in the audit log. Returns the id and whether it already
+/// existed.
+///
+/// `expected_id` is the id the caller encrypted the member's profile for (#3502): the
+/// existing membership's id, or a fresh one that becomes the new row's. An existing
+/// membership under any other id is `AcceptanceTargetChanged`, so a profile can never land
+/// on a row its associated data does not name; an erased one is `MembershipErased`.
 pub(crate) async fn ensure_membership(
     conn: &mut PgConnection,
     tenant_id: Uuid,
     account_id: Uuid,
+    expected_id: Uuid,
 ) -> Result<(Uuid, bool), MembershipError> {
-    let existing: Option<Uuid> = sqlx::query_scalar(
-        "select id from memberships where tenant_id = $1 and account_id = $2 for update",
+    let existing: Option<(Uuid, bool)> = sqlx::query_as(
+        "select id, name_erased_at is not null from memberships
+          where tenant_id = $1 and account_id = $2 for update",
     )
     .bind(tenant_id)
     .bind(account_id)
     .fetch_optional(&mut *conn)
     .await?;
-    if let Some(id) = existing {
+    if let Some((id, erased)) = existing {
+        if id != expected_id {
+            return Err(MembershipError::AcceptanceTargetChanged);
+        }
+        if erased {
+            return Err(MembershipError::MembershipErased);
+        }
         sqlx::query("update memberships set revoked_at = null where tenant_id = $1 and id = $2")
             .bind(tenant_id)
             .bind(id)
@@ -271,14 +285,13 @@
             .await?;
         return Ok((id, true));
     }
-    let id = Uuid::now_v7();
     sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1, $2, $3)")
         .bind(tenant_id)
-        .bind(id)
+        .bind(expected_id)
         .bind(account_id)
         .execute(&mut *conn)
         .await?;
-    Ok((id, false))
+    Ok((expected_id, false))
 }
 
 /// Inserts a role assignment and returns its id.
```

- [ ] **Step 5: Create `profile.rs`**

Create `backend/crates/persistence/src/membership/profile.rs`:

```rust
//! A member's directory fields (groups design §4.1; #3502): the display name, required when
//! an invitation is accepted or an FAU activated, and an optional contact address.
//!
//! **Both are content.** The caller encrypts each under the FAU's record key
//! (`fau_crypto::Unit::Record`) with `Aad::new(tenant_id, DISPLAY_NAME_AAD.0,
//! DISPLAY_NAME_AAD.1, membership_id)` or the same with [`CONTACT_EMAIL_AAD`]. This module
//! stores and returns the ciphertext only, and never writes a name or an address to audit,
//! to a NOTIFY or to a log.
//!
//! **The contact address is the member's own statement.** It is not verified, and no system
//! mail is ever sent to it: login, invitations and recovery keep using the account address.

use std::ops::RangeInclusive;

use fau_crypto::Ciphertext;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::MembershipError;

/// The associated data a display name is encrypted with, with the tenant and the
/// membership's id.
pub const DISPLAY_NAME_AAD: (&str, &str) = ("memberships", "encrypted_display_name");

/// The associated data a contact address is encrypted with, with the tenant and the
/// membership's id.
pub const CONTACT_EMAIL_AAD: (&str, &str) = ("memberships", "encrypted_contact_email");

/// The envelope sizes either field may have, the same octet bounds as migration 0008's
/// checks: fau-crypto's 41 bytes of version, nonce and tag around 1..=471 bytes. A name
/// (`DisplayName::MAX_CHARS` = 100 characters, at most 400 bytes) and an address
/// (`Email::MAX_LEN` = 254 bytes) both fit; the rest is headroom, not a second limit.
pub const MEMBER_FIELD_CIPHERTEXT_BYTES: RangeInclusive<usize> = 42..=512;

/// What a person gives when they join: their name, and optionally a contact address. Both
/// are bound to `membership_id`, which comes from `prepare_acceptance` (or, on activation,
/// is chosen fresh by the caller).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberProfile {
    pub membership_id: Uuid,
    pub encrypted_display_name: Ciphertext,
    pub encrypted_contact_email: Option<Ciphertext>,
}

fn well_formed(ct: &Ciphertext) -> bool {
    MEMBER_FIELD_CIPHERTEXT_BYTES.contains(&ct.len()) && ct.as_bytes()[0] == 1
}

pub(crate) fn check_display_name(ct: &Ciphertext) -> Result<(), MembershipError> {
    if well_formed(ct) {
        Ok(())
    } else {
        Err(MembershipError::DisplayNameMalformed)
    }
}

pub(crate) fn check_contact_email(ct: &Ciphertext) -> Result<(), MembershipError> {
    if well_formed(ct) {
        Ok(())
    } else {
        Err(MembershipError::ContactEmailMalformed)
    }
}

impl MemberProfile {
    pub(crate) fn check(&self) -> Result<(), MembershipError> {
        check_display_name(&self.encrypted_display_name)?;
        if let Some(ct) = &self.encrypted_contact_email {
            check_contact_email(ct)?;
        }
        Ok(())
    }
}

/// Writes both fields onto a membership that is not revoked (the caller has just created
/// or reopened it). A re-invited former member's row holds no name by then (D3: revocation
/// and the `clear_ended_profiles` sweep cleared it), so the new acceptance states the name
/// that applies from now on; one whose roles ran out before the sweep reached them has
/// theirs replaced.
pub(crate) async fn write_profile(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    profile: &MemberProfile,
) -> Result<(), MembershipError> {
    sqlx::query(
        "update memberships set encrypted_display_name = $3, encrypted_contact_email = $4
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id)
    .bind(profile.membership_id)
    .bind(profile.encrypted_display_name.as_bytes())
    .bind(
        profile
            .encrypted_contact_email
            .as_ref()
            .map(|c| c.as_bytes()),
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
```

- [ ] **Step 6: Take the profile at acceptance and activation, and clear both fields on revocation**

Apply to `backend/crates/persistence/src/membership/invitations.rs`:

```diff
--- a/backend/crates/persistence/src/membership/invitations.rs
+++ b/backend/crates/persistence/src/membership/invitations.rs
@@ -18,6 +18,7 @@
 use uuid::Uuid;
 
 use super::error::MembershipError;
+use super::profile::{write_profile, MemberProfile};
 use super::sql::{
     date_param, enqueue, ensure_membership, from_micros, handover_grant_valid, insert_assignment,
     is_admin_today, lock_tenant, membership_usable, period_from, recovery_notice_recipients,
@@ -720,6 +721,10 @@
     /// invitation offers (spec 3.4.3, 3.5), within the same 1–24 month range. Refused on
     /// any other mode.
     pub admin_end_override: Option<Date>,
+    /// The acceptor's display name, and optionally a contact address, encrypted for
+    /// `prepare_acceptance`'s membership id (groups design §4.1, #3502). Required: this is
+    /// where names enter the system.
+    pub profile: MemberProfile,
 }
 
 /// Hand-written so `token` never reaches a log line the way a derived `Debug` would.
@@ -729,6 +734,7 @@
             .field("token", &"[redacted]")
             .field("acceptor", &self.acceptor)
             .field("admin_end_override", &self.admin_end_override)
+            .field("profile", &self.profile)
             .finish()
     }
 }
@@ -755,6 +761,7 @@
     if !looks_like_token(&req.token) {
         return Err(MembershipError::UnknownInvitation);
     }
+    req.profile.check()?;
     let hash = hash_token(&req.token);
     let mut tx = pool.begin().await?;
 
@@ -868,7 +875,9 @@
     };
 
     let (account_id, _) = upsert_verified_account(&mut tx, &acceptor_email, at).await?;
-    let (membership_id, reused) = ensure_membership(&mut tx, tenant_id, account_id).await?;
+    let (membership_id, reused) =
+        ensure_membership(&mut tx, tenant_id, account_id, req.profile.membership_id).await?;
+    write_profile(&mut tx, tenant_id, &req.profile).await?;
     let mut assignment_ids = Vec::with_capacity(roles.len());
     for (role_id, _, period) in &roles {
         assignment_ids.push(
@@ -932,6 +941,54 @@
     })
 }
 
+/// Which FAU and which membership an acceptance will write to, so the caller can fetch that
+/// FAU's record key and encrypt the acceptor's profile for that membership's id before
+/// calling [`accept_invitation`] (#3502). The membership is the acceptor's existing one in
+/// that FAU -- a re-invited former member keeps theirs -- or a fresh UUIDv7 that becomes the
+/// new row's id.
+///
+/// Reads only. It answers for any invitation the token names, pending or not, as long as
+/// the verified address is its recipient: [`accept_invitation`] then gives the precise
+/// refusal (expired, withdrawn, already accepted). Every other failure is
+/// `UnknownInvitation`, so the answer never says which part failed.
+pub async fn prepare_acceptance(
+    pool: &PgPool,
+    token: &str,
+    acceptor: &VerifiedEmail,
+) -> Result<AcceptanceTarget, MembershipError> {
+    if !looks_like_token(token) {
+        return Err(MembershipError::UnknownInvitation);
+    }
+    let row: Option<(Uuid, String)> =
+        sqlx::query_as("select tenant_id, recipient_email from invitations where token_hash = $1")
+            .bind(hash_token(token))
+            .fetch_optional(pool)
+            .await?;
+    let (tenant_id, recipient) = row.ok_or(MembershipError::UnknownInvitation)?;
+    if recipient != acceptor.email().as_str() {
+        return Err(MembershipError::UnknownInvitation);
+    }
+    let existing: Option<Uuid> = sqlx::query_scalar(
+        "select m.id from memberships m join accounts a on a.id = m.account_id
+          where m.tenant_id = $1 and a.email = $2",
+    )
+    .bind(tenant_id)
+    .bind(acceptor.email().as_str())
+    .fetch_optional(pool)
+    .await?;
+    Ok(AcceptanceTarget {
+        tenant_id,
+        membership_id: existing.unwrap_or_else(Uuid::now_v7),
+    })
+}
+
+/// Where an acceptance will write: see [`prepare_acceptance`].
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub struct AcceptanceTarget {
+    pub tenant_id: Uuid,
+    pub membership_id: Uuid,
+}
+
 /// The invitee's view of the message (decision of 24 September 2026). Every failure is
 /// `UnknownInvitation`, so the answer never says which part failed.
 pub async fn invitation_message(
@@ -988,6 +1045,11 @@
             token: token.clone(),
             acceptor: VerifiedEmail::from_provider(Email::parse("ny@example.test").unwrap()),
             admin_end_override: None,
+            profile: MemberProfile {
+                membership_id: Uuid::now_v7(),
+                encrypted_display_name: fau_crypto::Ciphertext::from_stored(vec![1; 42]),
+                encrypted_contact_email: None,
+            },
         };
         let debug = format!("{req:?}");
         assert!(!debug.contains(&token));
```

Apply to `backend/crates/persistence/src/membership/signup.rs`:

```diff
--- a/backend/crates/persistence/src/membership/signup.rs
+++ b/backend/crates/persistence/src/membership/signup.rs
@@ -18,6 +18,7 @@
 
 use super::error::{ExistingFau, MembershipError};
 use super::invitations::{insert_invitation, IssuedInvitation, NewInvitation};
+use super::profile::{write_profile, MemberProfile};
 use super::sql::{
     date_param, enqueue, ensure_membership, from_micros, insert_assignment, lock_tenant,
     parse_date, ts_param, upsert_verified_account, write_audit, ActorKind, Audit,
@@ -310,6 +311,10 @@
     pub tenant_id: Uuid,
     /// The registrant's address, just verified with a Hanko passcode (#3417).
     pub registrant: VerifiedEmail,
+    /// The registrant's display name, and optionally a contact address, encrypted under the
+    /// FAU's record key for a fresh membership id the caller chose (#3502). The FAU is
+    /// pending, so no membership exists yet and any fresh id becomes the new row's.
+    pub profile: MemberProfile,
 }
 
 #[derive(Debug)]
@@ -333,6 +338,7 @@
     at: Moment,
 ) -> Result<Activated, MembershipError> {
     let tenant_id = activation.tenant_id;
+    activation.profile.check()?;
     let mut tx = pool.begin().await?;
     let state = lock_tenant(&mut tx, tenant_id).await?;
     if state.status != TenantStatus::Pending {
@@ -372,7 +378,14 @@
     if disabled {
         return Err(MembershipError::AccountDisabled);
     }
-    let (membership_id, _) = ensure_membership(&mut tx, tenant_id, account_id).await?;
+    let (membership_id, _) = ensure_membership(
+        &mut tx,
+        tenant_id,
+        account_id,
+        activation.profile.membership_id,
+    )
+    .await?;
+    write_profile(&mut tx, tenant_id, &activation.profile).await?;
 
     let admin_role_id = Uuid::now_v7();
     sqlx::query(
```

Apply to `backend/crates/persistence/src/membership/roles.rs`:

```diff
--- a/backend/crates/persistence/src/membership/roles.rs
+++ b/backend/crates/persistence/src/membership/roles.rs
@@ -255,7 +255,13 @@ pub async fn revoke_membership(
 
     let now = ts_param(at.now());
     sqlx::query(
-        "update memberships set revoked_at = $3::timestamptz where tenant_id = $1 and id = $2",
+        // Neither field outlives the membership (D3, 28 September 2026): history shows an
+        // ended membership as its role and year, never a name. Migration 0008's two
+        // *_only_while_current checks refuse a revocation that forgets either.
+        "update memberships
+            set revoked_at = $3::timestamptz,
+                encrypted_display_name = null, encrypted_contact_email = null
+          where tenant_id = $1 and id = $2",
     )
     .bind(req.tenant_id)
     .bind(req.membership_id)
```

Apply to `backend/crates/persistence/src/membership/mod.rs`:

```diff
--- a/backend/crates/persistence/src/membership/mod.rs
+++ b/backend/crates/persistence/src/membership/mod.rs
@@ -31,6 +31,7 @@
 mod groups;
 mod handover;
 mod invitations;
+mod profile;
 mod requests;
 mod roles;
 mod signup;
@@ -49,10 +50,13 @@
 };
 pub use handover::{create_handover_grants, recovery_grant_admin, RecoveryActor, RecoveryGrant};
 pub use invitations::{
-    accept_invitation, invitation_message, issue_invitation, resend_invitation,
-    withdraw_invitation, AcceptInvitation, Accepted, InvitationChange, InvitationMessage,
-    InvitationMessageView, IssueInvitation, IssuedInvitation, OfferedRole, RoleChoice,
-    INVITATION_MESSAGE_AAD,
+    accept_invitation, invitation_message, issue_invitation, prepare_acceptance, resend_invitation,
+    withdraw_invitation, AcceptInvitation, AcceptanceTarget, Accepted, InvitationChange,
+    InvitationMessage, InvitationMessageView, IssueInvitation, IssuedInvitation, OfferedRole,
+    RoleChoice, INVITATION_MESSAGE_AAD,
+};
+pub use profile::{
+    MemberProfile, CONTACT_EMAIL_AAD, DISPLAY_NAME_AAD, MEMBER_FIELD_CIPHERTEXT_BYTES,
 };
 pub use requests::{
     access_request_message, approve_request, create_access_request, create_replacement_proposal,
```

- [ ] **Step 7: Give every test caller a profile**

The fixtures first:

Apply to `backend/crates/app/tests/common/membership.rs`:

```diff
--- a/backend/crates/app/tests/common/membership.rs
+++ b/backend/crates/app/tests/common/membership.rs
@@ -1,14 +1,16 @@
 //! Builders for the membership integration tests. Every fixture goes through the
 //! persistence functions themselves, so a fixture that works is evidence too.
 
+use fau_crypto::Ciphertext;
 use fau_domain::email::{Email, VerifiedEmail};
 use fau_domain::membership::period::Period;
 use fau_domain::membership::rules::default_admin_end;
 use fau_domain::membership::vocabulary::{CapabilityClass, FauName, RoleName};
 use fau_domain::time::Moment;
 use fau_persistence::membership::{
-    accept_invitation, activate_tenant, create_pending_tenant, issue_invitation, AcceptInvitation,
-    Accepted, Activation, IssueInvitation, OfferedRole, PendingSignup, RoleChoice,
+    accept_invitation, activate_tenant, create_pending_tenant, issue_invitation,
+    prepare_acceptance, AcceptInvitation, Accepted, Activation, IssueInvitation, MemberProfile,
+    OfferedRole, PendingSignup, RoleChoice,
 };
 use jiff::civil::Date;
 use sqlx::PgPool;
@@ -66,6 +68,37 @@
     }
 }
 
+/// Shaped like fau-crypto's envelope (version byte 1, then 41 bytes), so migration 0008's
+/// checks accept it. Never decrypted: tests that decrypt use real keys (key_chain.rs).
+pub fn placeholder_envelope() -> Ciphertext {
+    let mut v = vec![1u8];
+    v.extend_from_slice(&[0u8; 41]);
+    Ciphertext::from_stored(v)
+}
+
+/// A profile for a membership that does not exist yet: any fresh id becomes the new row's.
+/// Enough for an activation, and for an acceptance by someone not yet in the FAU.
+pub fn fresh_profile() -> MemberProfile {
+    MemberProfile {
+        membership_id: Uuid::now_v7(),
+        encrypted_display_name: placeholder_envelope(),
+        encrypted_contact_email: None,
+    }
+}
+
+/// The profile an acceptance of `token` by `acceptor` needs: for `prepare_acceptance`'s
+/// membership, so a re-invited former member works too. A token `prepare_acceptance`
+/// refuses gets a fresh profile, so a test of the refusal still reaches `accept_invitation`.
+pub async fn profile_for(pool: &PgPool, token: &str, acceptor: &str) -> MemberProfile {
+    match prepare_acceptance(pool, token, &verified(acceptor)).await {
+        Ok(target) => MemberProfile {
+            membership_id: target.membership_id,
+            ..fresh_profile()
+        },
+        Err(_) => fresh_profile(),
+    }
+}
+
 /// An active FAU whose registrant is its only admin.
 pub struct Fau {
     pub tenant_id: Uuid,
@@ -87,6 +120,7 @@
         Activation {
             tenant_id: pending.tenant_id,
             registrant: verified(registrant),
+            profile: fresh_profile(),
         },
         at,
     )
@@ -132,12 +166,15 @@
     )
     .await
     .expect("issue");
+    let token = issued.token.expose().to_owned();
+    let profile = profile_for(pool, &token, address).await;
     accept_invitation(
         pool,
         AcceptInvitation {
-            token: issued.token.expose().to_owned(),
+            token,
             acceptor: verified(address),
             admin_end_override: None,
+            profile,
         },
         at,
     )
```

Then every `AcceptInvitation { .. }` and `Activation { .. }` literal in the test files gets a profile line after its last field. This is 22 insertions: 1 in `outbox.rs`, 1 in `guests.rs`, 1 in `requests.rs`, 1 in `handover_recovery.rs`'s `accept()` helper, 7 in `invitations.rs` (its `accept()` helper, the `with_end` closure, 3 literals and 2 `Activation`s) and 11 `Activation`s in `signup.rs`. The edit is mechanical, and this command makes exactly the 22 insertions:

```bash
cd /workspace/backend/crates/app/tests
for f in outbox.rs handover_recovery.rs guests.rs invitations.rs requests.rs signup.rs; do
  perl -0pi -e 's/^(\s*)(admin_end_override: [^\n]*,)\n/$1$2\n$1profile: fresh_profile(),\n/mg;
                s/^(\s*)(registrant: verified\([^\n]*\),)\n/$1$2\n$1profile: fresh_profile(),\n/mg' "$f"
done
grep -c 'profile: fresh_profile()' outbox.rs handover_recovery.rs guests.rs invitations.rs requests.rs signup.rs
cd /workspace/backend
```

Expected counts: `outbox.rs:1 handover_recovery.rs:1 guests.rs:1 invitations.rs:7 requests.rs:1 signup.rs:11`.

`fresh_profile()` names a fresh id, which is right for everyone joining for the first time. The one test that re-invites a removed member must encrypt for the reopened membership:

Apply to `backend/crates/app/tests/handover_recovery.rs`:

```diff
--- a/backend/crates/app/tests/handover_recovery.rs
+++ b/backend/crates/app/tests/handover_recovery.rs
@@ -1271,9 +1271,13 @@
     )
     .await
     .unwrap();
+    // A reopened membership keeps its id, so the profile is encrypted for it (#3502).
     let back = accept_invitation(
         &pool,
-        accept(issued.token.expose(), "admin@example.test"),
+        AcceptInvitation {
+            profile: profile_for(&pool, issued.token.expose(), "admin@example.test").await,
+            ..accept(issued.token.expose(), "admin@example.test")
+        },
         d,
     )
     .await
```

(The diff is against the file as the command above left it.)

Two tests sabotage a membership into the revoked state with raw SQL. Under D3 a revoked membership may hold no name (Task 3b's `memberships_display_name_only_while_current`), and every membership now has one, so each fixture clears the fields in the same statement. The `invitations.rs` diff is against the file as the perl command left it:

Apply to `backend/crates/app/tests/invitations.rs`:

```diff
--- a/backend/crates/app/tests/invitations.rs
+++ b/backend/crates/app/tests/invitations.rs
@@ -648,11 +648,16 @@
         t0,
     )
     .await;
-    sqlx::query("update memberships set revoked_at = now() where id = $1")
-        .bind(first.membership_id)
-        .execute(&db.admin_pool())
-        .await
-        .unwrap();
+    // D3: a revoked membership holds no name, so the sabotage clears it too (#3502).
+    sqlx::query(
+        "update memberships
+            set revoked_at = now(), encrypted_display_name = null, encrypted_contact_email = null
+          where id = $1",
+    )
+    .bind(first.membership_id)
+    .execute(&db.admin_pool())
+    .await
+    .unwrap();
 
     let again = add_member(
         &pool,
```

Apply to `backend/crates/app/tests/roles.rs`:

```diff
--- a/backend/crates/app/tests/roles.rs
+++ b/backend/crates/app/tests/roles.rs
@@ -532,13 +532,18 @@ async fn a_revoked_membership_with_an_unrevoked_admin_assignment_is_not_an_admin
     )
     .await;
     // No persistence function reaches this shape -- arranged directly with the
-    // superuser pool, per the global constraints (fix round 1, item 5).
-    sqlx::query("update memberships set revoked_at = now() where tenant_id = $1 and id = $2")
-        .bind(fau.tenant_id)
-        .bind(other.membership_id)
-        .execute(&admin_pool)
-        .await
-        .unwrap();
+    // superuser pool, per the global constraints (fix round 1, item 5). D3: a revoked
+    // membership holds no name, so the name goes with it (#3502).
+    sqlx::query(
+        "update memberships
+            set revoked_at = now(), encrypted_display_name = null, encrypted_contact_email = null
+          where tenant_id = $1 and id = $2",
+    )
+    .bind(fau.tenant_id)
+    .bind(other.membership_id)
+    .execute(&admin_pool)
+    .await
+    .unwrap();
     assert!(
         !revoked(&pool, "role_assignments", other.assignment_ids[0]).await,
         "the assignment itself stays unrevoked; only the membership was sabotaged"
```

The `roles.rs` fixture is the one reachable shape of a revoked membership whose assignments are not revoked. It is why Task 5's `membership_ended` tests `m.revoked_at` as well as the assignments.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test member_profile --test invitations --test handover_recovery --test signup --test guests --test requests --test outbox --test roles && cargo test -p fau-persistence --lib`
Expected: PASS. `member_profile` shows 7 passed. The six changed suites pass as before.

Mutation checks:
- In `ensure_membership`, drop the `id != expected_id` branch, and `a_profile_for_another_membership_is_refused` fails.
- In `prepare_acceptance`, drop `m.tenant_id = $1`, and `prepare_names_only_this_faus_membership_and_only_for_the_recipient` fails.
- Drop `encrypted_display_name = null` from `revoke_membership`, and `revoking_a_membership_clears_its_name_and_contact_address` fails on `memberships_display_name_only_while_current`. Drop `encrypted_contact_email = null`, and it fails on `memberships_contact_email_only_while_current`.

- [ ] **Step 9: Run the whole workspace once**

The accept path is shared, so run all of it: `ps aux | grep '[c]argo test'` (must be empty), then `cargo test --workspace`.
Expected: every suite passes (736 in the dry run at this state). `roles` includes the re-fixtured `a_revoked_membership_with_an_unrevoked_admin_assignment_is_not_an_admin`.

- [ ] **Step 10: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 11: Commit**

```bash
git add backend/crates/persistence/src/membership/ \
        backend/crates/app/tests/common/membership.rs \
        backend/crates/app/tests/member_profile.rs \
        backend/crates/app/tests/outbox.rs \
        backend/crates/app/tests/guests.rs \
        backend/crates/app/tests/invitations.rs \
        backend/crates/app/tests/requests.rs \
        backend/crates/app/tests/signup.rs \
        backend/crates/app/tests/handover_recovery.rs \
        backend/crates/app/tests/roles.rs
git commit -m "Capture the display name when an invitation is accepted or an FAU activated (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Editing a name and a contact address

**Files:**
- Modify: `backend/crates/persistence/src/membership/profile.rs`, `mod.rs`
- Test: `backend/crates/app/tests/profile_edits.rs`

**Interfaces:**
- Consumes:
  - `check_display_name` and `check_contact_email`, and `MembershipError::MembershipEnded` (Task 4);
  - `membership_access`, `is_admin_today`, `lock_tenant`, `require_open`, `date_param`, `write_audit` and `Audit::member`.
- Produces:
  - `pub(crate) membership_ended(date_param: &str) -> String`, the SQL predicate "membership `m` has ended" (revoked, or no assignment running or still to come), shared with Task 6 (Ruling D3-2);
  - `struct SetDisplayName { tenant_id, actor_membership_id, membership_id, encrypted_display_name: Ciphertext }` and `set_display_name(pool, SetDisplayName, Moment) -> Result<(), MembershipError>`, refusing an ended target with `MembershipEnded` (Ruling D3-6);
  - `struct SetContactEmail { tenant_id, membership_id, encrypted_contact_email: Option<Ciphertext> }` and `set_contact_email(pool, SetContactEmail, Moment) -> Result<(), MembershipError>`;
  - audit actions `membership.display_name_changed {by_admin}` and `membership.contact_email_changed {cleared}`.

- [ ] **Step 1: Write the failing test**

Create `backend/crates/app/tests/profile_edits.rs`:

```rust
//! Editing the directory fields (groups design §4.1; #3502): a member edits their own name,
//! an admin corrects that of anyone still active, and only the member sets their contact
//! address. An ended membership takes no name (D3). Every refusal a non-admin can reach is
//! the same `NotAuthorized`, whatever the target is.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

fn envelope(marker: u8) -> Ciphertext {
    let mut v = vec![1u8, marker];
    v.extend_from_slice(&[0u8; 40]);
    Ciphertext::from_stored(v)
}

fn year() -> fau_domain::membership::period::Period {
    period(day(2026, 9, 1), day(2027, 9, 1))
}

async fn member(pool: &PgPool, fau: &Fau, address: &str) -> Uuid {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        year(),
        at(T0),
    )
    .await
    .membership_id
}

async fn name_of(pool: &PgPool, m: Uuid) -> Option<Vec<u8>> {
    sqlx::query_scalar("select encrypted_display_name from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn contact_of(pool: &PgPool, m: Uuid) -> Option<Vec<u8>> {
    sqlx::query_scalar("select encrypted_contact_email from memberships where id = $1")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn rename(
    pool: &PgPool,
    tenant_id: Uuid,
    actor: Uuid,
    target: Uuid,
    marker: u8,
) -> Result<(), MembershipError> {
    set_display_name(
        pool,
        SetDisplayName {
            tenant_id,
            actor_membership_id: actor,
            membership_id: target,
            encrypted_display_name: envelope(marker),
        },
        at(T0),
    )
    .await
}

async fn last_audit(pool: &PgPool, action: &str) -> serde_json::Value {
    let params: String = sqlx::query_scalar(
        "select params::text from audit_events where action = $1 order by occurred_at desc, id desc limit 1",
    )
    .bind(action)
    .fetch_one(pool)
    .await
    .unwrap();
    serde_json::from_str(&params).unwrap()
}

#[tokio::test]
async fn a_member_edits_their_own_name_and_the_audit_holds_no_name() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    rename(&pool, fau.tenant_id, kari, kari, 42).await.unwrap();
    assert_eq!(
        name_of(&pool, kari).await.as_deref(),
        Some(envelope(42).as_bytes())
    );
    assert_eq!(
        last_audit(&pool, "membership.display_name_changed").await,
        serde_json::json!({ "by_admin": false })
    );
}

/// Mutation check: drop the `membership_ended` test from `set_display_name`, and the
/// revoked row fails on migration 0008's `memberships_display_name_only_while_current`
/// while the ran-out row is renamed.
#[tokio::test]
async fn an_admin_corrects_an_active_members_name_and_an_ended_membership_takes_none() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    rename(&pool, fau.tenant_id, fau.admin_membership_id, kari, 8)
        .await
        .unwrap();
    assert_eq!(
        name_of(&pool, kari).await.as_deref(),
        Some(envelope(8).as_bytes())
    );
    assert_eq!(
        last_audit(&pool, "membership.display_name_changed").await,
        serde_json::json!({ "by_admin": true })
    );

    // Active before it has begun: every role is still to come.
    let next_year = add_member(
        &pool,
        &fau,
        "neste@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 10, 1), day(2027, 9, 1)),
        at(T0),
    )
    .await
    .membership_id;
    rename(&pool, fau.tenant_id, fau.admin_membership_id, next_year, 6)
        .await
        .unwrap();

    // Revoked: the name went with it, and a correction cannot bring one back.
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: kari,
            confirm_no_admin: false,
        },
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(
        rename(&pool, fau.tenant_id, fau.admin_membership_id, kari, 9).await,
        Err(MembershipError::MembershipEnded)
    );
    assert_eq!(name_of(&pool, kari).await, None);

    // Ran out, and the sweep has not reached it yet: the old name stays untouched until
    // it does, and no new one is accepted.
    let month = add_member(
        &pool,
        &fau,
        "maaned@example.test",
        new_role("Medlem", CapabilityClass::Member),
        period(day(2026, 9, 1), day(2026, 10, 1)),
        at(T0),
    )
    .await
    .membership_id;
    let before = name_of(&pool, month).await;
    assert_eq!(
        set_display_name(
            &pool,
            SetDisplayName {
                tenant_id: fau.tenant_id,
                actor_membership_id: fau.admin_membership_id,
                membership_id: month,
                encrypted_display_name: envelope(9),
            },
            at("2026-10-01T10:00:00Z"),
        )
        .await,
        Err(MembershipError::MembershipEnded)
    );
    assert_eq!(name_of(&pool, month).await, before);

    // Only an admin learns that an id does not exist.
    assert_eq!(
        rename(
            &pool,
            fau.tenant_id,
            fau.admin_membership_id,
            Uuid::now_v7(),
            8
        )
        .await,
        Err(MembershipError::UnknownMembership)
    );
}

/// Mutation check: drop the `is_admin_today` branch, or the standing check on the own
/// path, and one of these rows passes.
#[tokio::test]
async fn nobody_else_may_edit_a_name_and_every_refusal_looks_the_same() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let other = active_fau(&pool, "admin-b@example.test", t0).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    let ola = member(&pool, &fau, "ola@example.test").await;
    let group = seed_group(
        &pool,
        &fau,
        fau_domain::membership::vocabulary::Visibility::Open,
    )
    .await;
    let guest = add_guest(&pool, &fau, "gjest@example.test", group, year(), t0)
        .await
        .membership_id;
    let ended = member(&pool, &fau, "ferdig@example.test").await;
    revoke(&pool, &fau, live_assignment(&pool, ended).await, t0).await;
    let outsider = member(&pool, &other, "utenfor@example.test").await;

    let before = name_of(&pool, kari).await;
    for (actor, target, why) in [
        (ola, kari, "a member, on another member"),
        (guest, kari, "a guest, on a member"),
        (ola, Uuid::now_v7(), "a member, on an unknown id"),
        (ola, outsider, "a member, on another FAU's membership"),
        (ended, ended, "no standing, on themself"),
        (outsider, kari, "another FAU's member, on this FAU's"),
    ] {
        assert_eq!(
            rename(&pool, fau.tenant_id, actor, target, 99).await,
            Err(MembershipError::NotAuthorized),
            "{why}"
        );
    }
    assert_eq!(name_of(&pool, kari).await, before);

    // Another FAU's admin cannot reach into this one, either way round.
    assert_eq!(
        rename(&pool, fau.tenant_id, other.admin_membership_id, kari, 99).await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        rename(&pool, other.tenant_id, other.admin_membership_id, kari, 99).await,
        Err(MembershipError::UnknownMembership)
    );
    assert_eq!(name_of(&pool, kari).await, before);

    // A guest may edit their own name.
    rename(&pool, fau.tenant_id, guest, guest, 5).await.unwrap();
}

#[tokio::test]
async fn a_disabled_account_edits_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = add_member(
        &pool,
        &fau,
        "kari@example.test",
        new_role("Medlem", CapabilityClass::Member),
        year(),
        at(T0),
    )
    .await;
    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(kari.account_id)
        .execute(&pool)
        .await
        .unwrap();
    let m = kari.membership_id;
    assert_eq!(
        rename(&pool, fau.tenant_id, m, m, 1).await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        set_contact_email(
            &pool,
            SetContactEmail {
                tenant_id: fau.tenant_id,
                membership_id: m,
                encrypted_contact_email: Some(envelope(1)),
            },
            at(T0),
        )
        .await,
        Err(MembershipError::NotAuthorized)
    );
}

#[tokio::test]
async fn only_the_member_sets_or_clears_their_contact_address() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    let set = |m: Uuid, v: Option<Ciphertext>| {
        let pool = pool.clone();
        async move {
            set_contact_email(
                &pool,
                SetContactEmail {
                    tenant_id: fau.tenant_id,
                    membership_id: m,
                    encrypted_contact_email: v,
                },
                at(T0),
            )
            .await
        }
    };
    set(kari, Some(envelope(4))).await.unwrap();
    assert_eq!(
        contact_of(&pool, kari).await.as_deref(),
        Some(envelope(4).as_bytes())
    );
    assert_eq!(
        last_audit(&pool, "membership.contact_email_changed").await,
        serde_json::json!({ "cleared": false })
    );
    set(kari, None).await.unwrap();
    assert_eq!(contact_of(&pool, kari).await, None);
    assert_eq!(
        last_audit(&pool, "membership.contact_email_changed").await,
        serde_json::json!({ "cleared": true })
    );
    assert_eq!(
        set(Uuid::now_v7(), Some(envelope(4))).await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        set(kari, Some(Ciphertext::from_stored(vec![1u8; 41]))).await,
        Err(MembershipError::ContactEmailMalformed)
    );
}

#[tokio::test]
async fn a_frozen_fau_takes_no_new_name_or_address_but_lets_one_be_cleared() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let fau = active_fau(&pool, "admin@example.test", at(T0)).await;
    let kari = member(&pool, &fau, "kari@example.test").await;
    set_contact_email(
        &pool,
        SetContactEmail {
            tenant_id: fau.tenant_id,
            membership_id: kari,
            encrypted_contact_email: Some(envelope(4)),
        },
        at(T0),
    )
    .await
    .unwrap();
    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(fau.tenant_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        rename(&pool, fau.tenant_id, kari, kari, 1).await,
        Err(MembershipError::TenantFrozen)
    );
    let write = |v: Option<Ciphertext>| {
        let pool = pool.clone();
        async move {
            set_contact_email(
                &pool,
                SetContactEmail {
                    tenant_id: fau.tenant_id,
                    membership_id: kari,
                    encrypted_contact_email: v,
                },
                at(T0),
            )
            .await
        }
    };
    assert_eq!(
        write(Some(envelope(5))).await,
        Err(MembershipError::TenantFrozen)
    );
    write(None).await.unwrap();
    assert_eq!(contact_of(&pool, kari).await, None);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p fau-app --test profile_edits`
Expected: compile errors, `cannot find function set_display_name` and `cannot find struct SetContactEmail`.

- [ ] **Step 3: Write the implementation**

Apply to `backend/crates/persistence/src/membership/profile.rs`:

```diff
--- a/backend/crates/persistence/src/membership/profile.rs
+++ b/backend/crates/persistence/src/membership/profile.rs
@@ -9,14 +9,26 @@
 //!
 //! **The contact address is the member's own statement.** It is not verified, and no system
 //! mail is ever sent to it: login, invitations and recovery keep using the account address.
+//! So only the member sets it. An admin may correct a name, not an address.
+//!
+//! **Neither field outlives the membership** (Erik's D3, 28 September 2026). Once a
+//! membership has ended ([`membership_ended`]) it holds no name, so no edit gives it one;
+//! history shows it as its role and year instead (`member_names`).
+//!
+//! **Order** (as in the rest of `membership`): tenant state, then authority, then row state.
 
 use std::ops::RangeInclusive;
 
 use fau_crypto::Ciphertext;
-use sqlx::PgConnection;
+use fau_domain::membership::access::Capability;
+use fau_domain::time::Moment;
+use serde_json::json;
+use sqlx::{PgConnection, PgPool};
 use uuid::Uuid;
 
+use super::access::membership_access;
 use super::error::MembershipError;
+use super::sql::{date_param, is_admin_today, lock_tenant, require_open, write_audit, Audit};
 
 /// The associated data a display name is encrypted with, with the tenant and the
 /// membership's id.
@@ -99,3 +111,159 @@ pub(crate) async fn write_profile(
     .await?;
     Ok(())
 }
+
+#[derive(Debug, Clone)]
+pub struct SetDisplayName {
+    pub tenant_id: Uuid,
+    pub actor_membership_id: Uuid,
+    /// Equal to `actor_membership_id` when a member edits their own name.
+    pub membership_id: Uuid,
+    pub encrypted_display_name: Ciphertext,
+}
+
+/// SQL: membership `m` has ended on the date bound as `date_param` (for example `$3`). It
+/// is revoked, or none of its role assignments is still running or yet to start (plan R8).
+/// A handover grant does not keep a membership going: it is a recovery right, not a role.
+///
+/// The one definition that editing ([`set_display_name`]), the retention sweep
+/// (`clear_ended_profiles`) and history (`member_names`) share (D3). `revoked_at` is tested
+/// on its own too: `revoke_membership` revokes the running and future assignments with the
+/// membership, but a row revoked any other way must still count as ended.
+pub(crate) fn membership_ended(date_param: &str) -> String {
+    format!(
+        "(m.revoked_at is not null
+          or not exists (select 1 from role_assignments ra
+                          where ra.tenant_id = m.tenant_id and ra.membership_id = m.id
+                            and ra.revoked_at is null
+                            and ra.ends_on_exclusive > {date_param}::date))"
+    )
+}
+
+/// A member edits their own name, or an admin corrects the name of anyone whose membership
+/// is still active (§4.1). Refused while the FAU is frozen, for a name an Article 17
+/// erasure removed (`MembershipErased`), and for a membership that has ended
+/// (`MembershipEnded`): under D3 an ended membership holds no name, and history shows its
+/// role and year instead. A membership whose roles are all still to come is active.
+///
+/// **Authority before state:** a member naming anyone but themselves gets `NotAuthorized`
+/// whether or not the id exists; only an admin learns `UnknownMembership` or
+/// `MembershipEnded`.
+pub async fn set_display_name(
+    pool: &PgPool,
+    req: SetDisplayName,
+    at: Moment,
+) -> Result<(), MembershipError> {
+    check_display_name(&req.encrypted_display_name)?;
+    let mut tx = pool.begin().await?;
+    let state = lock_tenant(&mut tx, req.tenant_id).await?;
+    require_open(&state)?;
+    let own = req.actor_membership_id == req.membership_id;
+    let by_admin = if own {
+        let access =
+            membership_access(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await?;
+        if access.capability == Capability::None {
+            return Err(MembershipError::NotAuthorized);
+        }
+        false
+    } else {
+        if !is_admin_today(&mut tx, req.tenant_id, req.actor_membership_id, at.today()).await? {
+            return Err(MembershipError::NotAuthorized);
+        }
+        true
+    };
+    let row: Option<(bool, bool)> = sqlx::query_as(&format!(
+        "select m.name_erased_at is not null, {}
+           from memberships m
+          where m.tenant_id = $1 and m.id = $2 for update",
+        membership_ended("$3")
+    ))
+    .bind(req.tenant_id)
+    .bind(req.membership_id)
+    .bind(date_param(at.today()))
+    .fetch_optional(&mut *tx)
+    .await?;
+    match row {
+        None => return Err(MembershipError::UnknownMembership),
+        Some((true, _)) => return Err(MembershipError::MembershipErased),
+        Some((false, true)) => return Err(MembershipError::MembershipEnded),
+        Some((false, false)) => {}
+    }
+    sqlx::query(
+        "update memberships set encrypted_display_name = $3 where tenant_id = $1 and id = $2",
+    )
+    .bind(req.tenant_id)
+    .bind(req.membership_id)
+    .bind(req.encrypted_display_name.as_bytes())
+    .execute(&mut *tx)
+    .await?;
+    write_audit(
+        &mut tx,
+        at,
+        Audit::member(
+            req.tenant_id,
+            req.actor_membership_id,
+            "membership.display_name_changed",
+            "membership",
+            req.membership_id,
+            json!({ "by_admin": by_admin }),
+        ),
+    )
+    .await?;
+    tx.commit().await?;
+    Ok(())
+}
+
+#[derive(Debug, Clone)]
+pub struct SetContactEmail {
+    pub tenant_id: Uuid,
+    /// Only the member themself: the address is their own statement (§4.1).
+    pub membership_id: Uuid,
+    /// `None` clears it, and the login address is shown instead.
+    pub encrypted_contact_email: Option<Ciphertext>,
+}
+
+/// A member sets or clears their own contact address. Setting one needs an open FAU;
+/// clearing one reduces what others see, so it is allowed while frozen. Either needs
+/// standing today: someone whose roles have ended has no address to show, and the
+/// retention sweep clears it.
+pub async fn set_contact_email(
+    pool: &PgPool,
+    req: SetContactEmail,
+    at: Moment,
+) -> Result<(), MembershipError> {
+    if let Some(ct) = &req.encrypted_contact_email {
+        check_contact_email(ct)?;
+    }
+    let mut tx = pool.begin().await?;
+    let state = lock_tenant(&mut tx, req.tenant_id).await?;
+    if req.encrypted_contact_email.is_some() {
+        require_open(&state)?;
+    }
+    let access = membership_access(&mut tx, req.tenant_id, req.membership_id, at.today()).await?;
+    if access.capability == Capability::None {
+        return Err(MembershipError::NotAuthorized);
+    }
+    sqlx::query(
+        "update memberships set encrypted_contact_email = $3 where tenant_id = $1 and id = $2",
+    )
+    .bind(req.tenant_id)
+    .bind(req.membership_id)
+    .bind(req.encrypted_contact_email.as_ref().map(|c| c.as_bytes()))
+    .execute(&mut *tx)
+    .await?;
+    write_audit(
+        &mut tx,
+        at,
+        Audit::member(
+            req.tenant_id,
+            req.membership_id,
+            "membership.contact_email_changed",
+            "membership",
+            req.membership_id,
+            json!({ "cleared": req.encrypted_contact_email.is_none() }),
+        ),
+    )
+    .await?;
+    tx.commit().await?;
+    Ok(())
+}
```

Apply to `backend/crates/persistence/src/membership/mod.rs`:

```diff
--- a/backend/crates/persistence/src/membership/mod.rs
+++ b/backend/crates/persistence/src/membership/mod.rs
@@ -56,7 +56,8 @@
     RoleChoice, INVITATION_MESSAGE_AAD,
 };
 pub use profile::{
-    MemberProfile, CONTACT_EMAIL_AAD, DISPLAY_NAME_AAD, MEMBER_FIELD_CIPHERTEXT_BYTES,
+    set_contact_email, set_display_name, MemberProfile, SetContactEmail, SetDisplayName,
+    CONTACT_EMAIL_AAD, DISPLAY_NAME_AAD, MEMBER_FIELD_CIPHERTEXT_BYTES,
 };
 pub use requests::{
     access_request_message, approve_request, create_access_request, create_replacement_proposal,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test profile_edits`
Expected: PASS, 6 tests.

Mutation checks:
- Drop the `is_admin_today` branch (let any actor through), and `nobody_else_may_edit_a_name_and_every_refusal_looks_the_same` fails on its first row.
- Drop the standing check on the own path, and its `no standing` row fails.
- Drop `tenant_id = $1` from the row lookup, and the cross-FAU admin row returns `Ok` instead of `UnknownMembership`.
- Let an ended membership through (`Some((false, true)) => {}`), and `an_admin_corrects_an_active_members_name_and_an_ended_membership_takes_none` fails: the revoked row hits `memberships_display_name_only_while_current`, and the ran-out row is renamed. This was run in the dry run.

- [ ] **Step 5: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/persistence/src/membership/profile.rs \
        backend/crates/persistence/src/membership/mod.rs \
        backend/crates/app/tests/profile_edits.rs
git commit -m "Let members edit their name and contact address, and admins correct names (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Retention: the ended-membership sweep, erasure, and history's role-and-year label

Rewritten 28 September 2026 for D3. A name no longer survives the end of a membership: the sweep clears it with the contact address, and history shows an ended membership as the role it held and its years (Rulings D3-2 to D3-5).

**Files:**
- Create: `backend/crates/domain/src/directory/history.rs` (unit tests inside)
- Modify: `backend/crates/domain/src/directory/mod.rs`
- Create: `backend/crates/persistence/src/membership/retention.rs`
- Create: `backend/crates/persistence/src/membership/names.rs`
- Modify: `backend/crates/persistence/src/membership/mod.rs`
- Test: `backend/crates/app/tests/directory_retention.rs`

**Interfaces:**
- Consumes:
  - `membership_ended` (Task 5);
  - `authorize(.., Resource::Fau, Action::Read, ..)` and `denied`, `read_transaction` and `lock_tenant`;
  - `Audit::system`; `parse_date`, `from_micros` and `fau_domain::time::oslo_today`;
  - `MembershipError::MembershipErased` (Task 4), which `ensure_membership` and `set_display_name` already return for an erased row.
- Produces:
  - in `fau_domain::directory::history`:
    - `struct HeldRole { name: String, class: CapabilityClass, from: Date, until: Date }` (the days actually held, `until` exclusive; redacted `Debug`);
    - `struct RoleLabel { role: String, first_year: i16, last_year: i16 }` with `is_one_year()` (redacted `Debug`);
    - `role_label(&[HeldRole], on: Date) -> Option<RoleLabel>`;
  - `clear_ended_profiles(pool, Moment) -> Result<u64, MembershipError>`;
  - `erase_member_names(pool, account_id: Uuid, Moment) -> Result<u64, MembershipError>`;
  - `enum MemberName { Named(Ciphertext), Unnamed, Ended(Vec<HeldRole>), Former }`;
  - `member_names(pool, Viewer, &[Uuid], Moment) -> Result<Vec<(Uuid, MemberName)>, MembershipError>`;
  - audit actions `membership.profile_cleared {cause: "membership_ended"}` and `membership.name_erased {}`, both by the system actor.

- [ ] **Step 1: Write the failing tests**

Create `backend/crates/app/tests/directory_retention.rs`:

```rust
//! How long the directory fields live (groups design §4.2 as amended by Erik's D3, 28
//! September 2026; §10; #3502): neither the name nor the contact address outlives the
//! membership, history shows an ended membership as its role and years, and an erasure
//! shows "Tidligere medlem".

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::directory::history::HeldRole;
use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::*;
use jiff::civil::Date;
use sqlx::PgPool;
use uuid::Uuid;

fn envelope(marker: u8) -> Ciphertext {
    let mut v = vec![1u8, marker];
    v.extend_from_slice(&[0u8; 40]);
    Ciphertext::from_stored(v)
}

async fn join(pool: &PgPool, fau: &Fau, address: &str, p: Period) -> Accepted {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        p,
        at(T0),
    )
    .await
}

async fn with_contact(pool: &PgPool, fau: &Fau, m: Uuid, marker: u8) {
    set_contact_email(
        pool,
        SetContactEmail {
            tenant_id: fau.tenant_id,
            membership_id: m,
            encrypted_contact_email: Some(envelope(marker)),
        },
        at(T0),
    )
    .await
    .unwrap();
}

async fn fields(pool: &PgPool, m: Uuid) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    sqlx::query_as(
        "select encrypted_display_name, encrypted_contact_email from memberships where id = $1",
    )
    .bind(m)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn name(pool: &PgPool, fau: &Fau, m: Uuid, at_: &str) -> MemberName {
    let viewer = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: fau.admin_membership_id,
    };
    let names = member_names(pool, viewer, &[m], at(at_)).await.unwrap();
    assert_eq!(names.len(), 1);
    names.into_iter().next().unwrap().1
}

/// "Medlem", held from `from` up to (not including) `until`.
fn medlem(from: Date, until: Date) -> MemberName {
    MemberName::Ended(vec![HeldRole {
        name: "Medlem".into(),
        class: CapabilityClass::Member,
        from,
        until,
    }])
}

/// Mutation checks: drop `ra.tenant_id = m.tenant_id` from `membership_ended` and Kari's
/// running role in FAU B keeps her FAU A fields; drop `ra.revoked_at is null` and the
/// stepped-down member keeps theirs; make `member_names` ignore `ended` and the read before
/// the sweep returns the name.
#[tokio::test]
async fn the_sweep_clears_the_name_and_address_once_no_role_runs_or_is_still_to_come() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    let short = period(day(2026, 9, 1), day(2026, 10, 1));
    let long = period(day(2026, 9, 1), day(2027, 9, 1));

    let ends = join(&pool, &a, "ends@example.test", short)
        .await
        .membership_id;
    // A name and no contact address: the sweep takes the name all the same.
    let quiet = join(&pool, &a, "quiet@example.test", short)
        .await
        .membership_id;
    let renewed = join(&pool, &a, "renewed@example.test", short)
        .await
        .membership_id;
    grant_role(
        &pool,
        GrantRole {
            tenant_id: a.tenant_id,
            actor_membership_id: a.admin_membership_id,
            membership_id: renewed,
            role: new_role("Neste år", CapabilityClass::Member),
            period: period(day(2026, 11, 1), day(2027, 11, 1)),
        },
        t0,
    )
    .await
    .unwrap();
    // The same person in two FAU-er: her role in A ends, her role in B runs on.
    let kari_a = join(&pool, &a, "kari@example.test", short)
        .await
        .membership_id;
    let kari_b = join(&pool, &b, "kari@example.test", long)
        .await
        .membership_id;
    // A role revoked early counts as ended.
    let stepped_down = join(&pool, &a, "down@example.test", long)
        .await
        .membership_id;
    for (fau, m, marker) in [
        (&a, ends, 1),
        (&a, renewed, 2),
        (&a, kari_a, 3),
        (&b, kari_b, 4),
        (&a, stepped_down, 5),
    ] {
        with_contact(&pool, fau, m, marker).await;
    }
    revoke(&pool, &a, live_assignment(&pool, stepped_down).await, t0).await;

    // While the short roles run, the sweep clears only the one whose role was revoked, and
    // history shows it as the role it held until the revocation day (T0, 23 September).
    assert_eq!(
        clear_ended_profiles(&pool, at("2026-09-30T10:00:00Z"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(fields(&pool, stepped_down).await, (None, None));
    assert_eq!(
        name(&pool, &a, stepped_down, "2026-09-30T10:00:00Z").await,
        medlem(day(2026, 9, 1), day(2026, 9, 23))
    );
    assert!(fields(&pool, ends).await.0.is_some());

    // The day the short roles end, history already shows the role, before any sweep.
    let after = at("2026-10-01T10:00:00Z");
    assert!(fields(&pool, ends).await.0.is_some(), "not swept yet");
    assert_eq!(
        name(&pool, &a, ends, "2026-10-01T10:00:00Z").await,
        medlem(day(2026, 9, 1), day(2026, 10, 1))
    );

    // Then the sweep clears every membership without a running or future role.
    assert_eq!(clear_ended_profiles(&pool, after).await.unwrap(), 3);
    for gone in [ends, quiet, kari_a] {
        assert_eq!(fields(&pool, gone).await, (None, None));
    }
    assert_eq!(
        fields(&pool, renewed).await,
        (
            Some(placeholder_envelope().as_bytes().to_vec()),
            Some(envelope(2).as_bytes().to_vec())
        ),
        "a role still to come keeps both"
    );
    assert_eq!(
        fields(&pool, kari_b).await,
        (
            Some(placeholder_envelope().as_bytes().to_vec()),
            Some(envelope(4).as_bytes().to_vec())
        ),
        "another FAU's membership is its own"
    );
    assert_eq!(
        name(&pool, &a, renewed, "2026-10-01T10:00:00Z").await,
        MemberName::Named(placeholder_envelope())
    );
    // Idempotent.
    assert_eq!(clear_ended_profiles(&pool, after).await.unwrap(), 0);

    let causes: Vec<String> = sqlx::query_scalar(
        "select params->>'cause' from audit_events
          where action = 'membership.profile_cleared' and actor_kind = 'system'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(causes, ["membership_ended"; 4]);
}

#[tokio::test]
async fn erasure_shows_former_member_in_every_fau_even_after_the_membership_ended() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));
    let in_a = join(&pool, &a, "kari@example.test", year).await;
    let in_b = join(
        &pool,
        &b,
        "kari@example.test",
        period(day(2026, 9, 1), day(2026, 10, 1)),
    )
    .await;
    let ola = join(&pool, &a, "ola@example.test", year)
        .await
        .membership_id;
    with_contact(&pool, &a, in_a.membership_id, 1).await;
    // Before the erasure, Kari's ended membership in B shows its role and years.
    assert_eq!(
        name(&pool, &b, in_b.membership_id, "2026-10-02T10:00:00Z").await,
        medlem(day(2026, 9, 1), day(2026, 10, 1))
    );

    assert_eq!(
        erase_member_names(&pool, in_a.account_id, t0)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        name(&pool, &a, in_a.membership_id, T0).await,
        MemberName::Former
    );
    assert_eq!(
        name(&pool, &b, in_b.membership_id, "2026-10-02T10:00:00Z").await,
        MemberName::Former,
        "not even the role and year"
    );
    assert_eq!(fields(&pool, in_a.membership_id).await, (None, None));
    assert_eq!(
        name(&pool, &a, ola, T0).await,
        MemberName::Named(placeholder_envelope()),
        "nobody else's name"
    );
    assert_eq!(
        erase_member_names(&pool, in_a.account_id, t0)
            .await
            .unwrap(),
        0
    );
    let audits: Vec<String> = sqlx::query_scalar(
        "select params::text from audit_events where action = 'membership.name_erased'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(audits, ["{}", "{}"]);

    // A correction cannot bring the name back, and neither can a new acceptance.
    assert_eq!(
        set_display_name(
            &pool,
            SetDisplayName {
                tenant_id: a.tenant_id,
                actor_membership_id: a.admin_membership_id,
                membership_id: in_a.membership_id,
                encrypted_display_name: envelope(9),
            },
            t0,
        )
        .await,
        Err(MembershipError::MembershipErased)
    );
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: a.tenant_id,
            actor_membership_id: a.admin_membership_id,
            membership_id: in_a.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    let token = issue_invitation(
        &pool,
        IssueInvitation {
            tenant_id: a.tenant_id,
            actor_membership_id: a.admin_membership_id,
            recipient: email("kari@example.test"),
            roles: vec![OfferedRole {
                role: new_role("Medlem", CapabilityClass::Member),
                period: year,
            }],
            handover_grant_id: None,
            message: None,
        },
        t0,
    )
    .await
    .unwrap()
    .token
    .expose()
    .to_owned();
    let profile = profile_for(&pool, &token, "kari@example.test").await;
    assert_eq!(profile.membership_id, in_a.membership_id);
    assert_eq!(
        accept_invitation(
            &pool,
            AcceptInvitation {
                token,
                acceptor: verified("kari@example.test"),
                admin_end_override: None,
                profile,
            },
            t0,
        )
        .await
        .unwrap_err(),
        MembershipError::MembershipErased
    );
}

#[tokio::test]
async fn names_for_history_are_for_members_and_admins_of_that_fau_only() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    let year = period(day(2026, 9, 1), day(2027, 9, 1));
    let kari = join(&pool, &a, "kari@example.test", year)
        .await
        .membership_id;
    // Active with no standing yet: every role is still to come, so the name applies.
    let next = join(
        &pool,
        &a,
        "neste@example.test",
        period(day(2026, 10, 1), day(2027, 9, 1)),
    )
    .await
    .membership_id;
    let in_b = join(&pool, &b, "ola@example.test", year)
        .await
        .membership_id;
    let group = seed_group(&pool, &a, Visibility::Open).await;
    let guest = add_guest(&pool, &a, "gjest@example.test", group, year, t0)
        .await
        .membership_id;

    let viewer = |m: Uuid| Viewer {
        tenant_id: a.tenant_id,
        membership_id: m,
    };
    // Another FAU's id, an unknown id and a repeat are left out.
    let names = member_names(
        &pool,
        viewer(kari),
        &[kari, in_b, Uuid::now_v7(), kari, next],
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        names,
        [
            (kari, MemberName::Named(placeholder_envelope())),
            (next, MemberName::Named(placeholder_envelope()))
        ]
    );
    assert_eq!(
        member_names(&pool, viewer(guest), &[kari], t0).await,
        Err(MembershipError::NotAuthorized)
    );
    assert_eq!(
        member_names(&pool, viewer(in_b), &[kari], t0).await,
        Err(MembershipError::NotAuthorized),
        "a viewer from another FAU"
    );
}
```

Create `backend/crates/domain/src/directory/history.rs` with only its `#[cfg(test)] mod tests { .. }` block for now (the whole file is in Step 3), and register it:

Apply to `backend/crates/domain/src/directory/mod.rs`:

```diff
--- a/backend/crates/domain/src/directory/mod.rs
+++ b/backend/crates/domain/src/directory/mod.rs
@@ -8,4 +8,5 @@
 
 pub mod address;
 pub mod collation;
+pub mod history;
 pub mod listing;
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p fau-app --test directory_retention` and `cargo test -p fau-domain --lib -- directory::history`
Expected: compile errors, `cannot find function clear_ended_profiles`, `erase_member_names` and `member_names`, `cannot find type MemberName` and `unresolved import fau_domain::directory::history::HeldRole`; in the domain, `cannot find type HeldRole` and `cannot find function role_label`.

- [ ] **Step 3: Write the implementation**

Create `backend/crates/domain/src/directory/history.rs`:

```rust
//! How history shows someone whose membership has ended (Erik's D3, 28 September 2026).
//!
//! A display name exists only while the membership is active. Once it ends, history -- an
//! author, a minute-taker, an audit line -- shows the role the person held and its years,
//! "Leder 2025–2026", never a name. Persistence (`member_names`) hands over the roles the
//! membership actually held, read from `role_assignments` when history is rendered, and
//! [`role_label`] picks the one that fits the moment being shown.
//!
//! **The rule** (Rulings D3-3 and D3-5):
//! - the role held on the day of the event; among several held that day, an admin-class
//!   role before a member-class one before a guest's, then the one that started first;
//! - an event after every role ended shows the last role that ended, and an event before any
//!   role began shows the first one to begin;
//! - one role only, never a list, and never the unit or cohort it sits on, since a place
//!   narrows a role down to a person;
//! - the years are the calendar years of the days the assignment was actually held, from
//!   its first day to its last.
//!
//! The screen renders a [`RoleLabel`] through the catalogue (#3439), never as prose built
//! here. The proposed Bokmål source strings are `member.endedRole.oneYear` = "{role} {year}"
//! and `member.endedRole.years` = "{role} {firstYear}–{lastYear}", with the years passed as
//! strings so ICU does not group their digits.

use std::fmt;

use jiff::civil::Date;

use crate::membership::vocabulary::CapabilityClass;

/// One role a membership held, over the days it was actually held: from `from` up to, but
/// not including, `until`. An assignment revoked early ends where it was revoked. Built
/// only for non-empty spans, so `from < until`. `Debug` is hand-written: `name` is a role
/// name, which is never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct HeldRole {
    pub name: String,
    pub class: CapabilityClass,
    pub from: Date,
    pub until: Date,
}

impl fmt::Debug for HeldRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeldRole")
            .field("name", &"[redacted]")
            .field("class", &self.class)
            .field("from", &self.from)
            .field("until", &self.until)
            .finish()
    }
}

/// What history shows for an ended membership: a role and its years. `Debug` is
/// hand-written for the same reason as [`HeldRole`]'s.
#[derive(Clone, PartialEq, Eq)]
pub struct RoleLabel {
    pub role: String,
    pub first_year: i16,
    pub last_year: i16,
}

impl RoleLabel {
    /// Whether the catalogue's one-year form applies ("Kasserer 2026") rather than the span
    /// ("Leder 2025–2026").
    pub fn is_one_year(&self) -> bool {
        self.first_year == self.last_year
    }
}

impl fmt::Debug for RoleLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoleLabel")
            .field("role", &"[redacted]")
            .field("first_year", &self.first_year)
            .field("last_year", &self.last_year)
            .finish()
    }
}

fn rank(class: CapabilityClass) -> u8 {
    match class {
        CapabilityClass::Admin => 0,
        CapabilityClass::Member => 1,
        CapabilityClass::Guest => 2,
    }
}

/// The label for an event on `on`, from the roles an ended membership held. `None` when it
/// held none, which the screen shows as "Tidligere medlem".
pub fn role_label(held: &[HeldRole], on: Date) -> Option<RoleLabel> {
    let during = held
        .iter()
        .filter(|r| r.from <= on && on < r.until)
        .min_by_key(|r| (rank(r.class), r.from));
    let after = || {
        held.iter()
            .filter(|r| r.until <= on)
            .min_by_key(|r| (std::cmp::Reverse(r.until), rank(r.class), r.from))
    };
    let before = || held.iter().min_by_key(|r| (r.from, rank(r.class)));
    let chosen = during.or_else(after).or_else(before)?;
    let last_day = chosen.until.yesterday().unwrap_or(chosen.from);
    Some(RoleLabel {
        role: chosen.name.clone(),
        first_year: chosen.from.year(),
        last_year: last_day.year(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn held(name: &str, class: CapabilityClass, from: Date, until: Date) -> HeldRole {
        HeldRole {
            name: name.to_owned(),
            class,
            from,
            until,
        }
    }

    fn shown(held: &[HeldRole], on: Date) -> Option<(String, i16, i16)> {
        role_label(held, on).map(|l| (l.role, l.first_year, l.last_year))
    }

    fn two_years() -> Vec<HeldRole> {
        vec![
            held(
                "Kasserer",
                CapabilityClass::Member,
                date(2024, 9, 1),
                date(2025, 9, 1),
            ),
            held(
                "Leder",
                CapabilityClass::Admin,
                date(2025, 9, 1),
                date(2026, 9, 1),
            ),
        ]
    }

    #[test]
    fn the_role_held_on_the_day_is_shown_with_its_years() {
        let h = two_years();
        assert_eq!(
            shown(&h, date(2025, 1, 10)),
            Some(("Kasserer".into(), 2024, 2025))
        );
        assert_eq!(
            shown(&h, date(2026, 3, 1)),
            Some(("Leder".into(), 2025, 2026))
        );
        // The first day belongs to the new role, the day before to the old one.
        assert_eq!(
            shown(&h, date(2025, 9, 1)),
            Some(("Leder".into(), 2025, 2026))
        );
        assert_eq!(
            shown(&h, date(2025, 8, 31)),
            Some(("Kasserer".into(), 2024, 2025))
        );
    }

    #[test]
    fn outside_every_role_the_nearest_end_or_the_first_start_is_shown() {
        let h = two_years();
        assert_eq!(
            shown(&h, date(2027, 1, 1)),
            Some(("Leder".into(), 2025, 2026)),
            "after the end: the last role that ended"
        );
        assert_eq!(
            shown(&h, date(2024, 1, 1)),
            Some(("Kasserer".into(), 2024, 2025)),
            "before any start: the first role to begin"
        );
    }

    #[test]
    fn on_one_day_an_admin_role_wins_then_the_earliest_start() {
        let h = vec![
            held(
                "Kontaktforelder",
                CapabilityClass::Member,
                date(2025, 8, 1),
                date(2026, 8, 1),
            ),
            held(
                "Leder",
                CapabilityClass::Admin,
                date(2025, 9, 1),
                date(2026, 9, 1),
            ),
            held(
                "Dugnadsansvarlig",
                CapabilityClass::Member,
                date(2025, 7, 1),
                date(2026, 7, 1),
            ),
        ];
        assert_eq!(shown(&h, date(2025, 10, 1)).unwrap().0, "Leder");
        assert_eq!(
            shown(&h, date(2025, 8, 15)).unwrap().0,
            "Dugnadsansvarlig",
            "two member roles: the one that started first"
        );
    }

    #[test]
    fn a_role_within_one_calendar_year_has_one_year() {
        let whole = [held(
            "Sekretær",
            CapabilityClass::Member,
            date(2026, 1, 1),
            date(2027, 1, 1),
        )];
        let label = role_label(&whole, date(2026, 6, 1)).unwrap();
        assert_eq!((label.first_year, label.last_year), (2026, 2026));
        assert!(label.is_one_year());
        let short = [held(
            "Medlem",
            CapabilityClass::Member,
            date(2026, 9, 1),
            date(2026, 10, 1),
        )];
        assert!(role_label(&short, date(2026, 9, 2)).unwrap().is_one_year());
        assert!(!role_label(&two_years(), date(2026, 3, 1))
            .unwrap()
            .is_one_year());
    }

    #[test]
    fn nothing_held_is_no_label() {
        assert_eq!(shown(&[], date(2026, 1, 1)), None);
    }

    #[test]
    fn debug_never_prints_a_role_name() {
        let h = two_years();
        let label = role_label(&h, date(2026, 3, 1)).unwrap();
        assert!(!format!("{h:?}").contains("Leder"));
        assert!(!format!("{h:?}").contains("Kasserer"));
        assert!(!format!("{label:?}").contains("Leder"));
    }
}
```

Create `backend/crates/persistence/src/membership/retention.rs`:

```rust
//! How long the directory fields live (groups design §4.2 as amended by Erik's D3, 28
//! September 2026; #3502).
//!
//! - **Neither field outlives the membership.** A display name is valid only while the
//!   membership is active; once it has ended (`membership_ended`: revoked, or no role
//!   assignment still running or yet to start) the name goes, exactly like the contact
//!   address. History then shows the membership as its role and year, computed at read time
//!   (`member_names`), so there is no name history to keep.
//! - Revocation clears both fields in the same statement (`revoke_membership`, held by
//!   migration 0008's two `*_only_while_current` checks). A membership whose roles simply ran
//!   out is not revoked, so [`clear_ended_profiles`], a daily sweep, clears it. Until the
//!   sweep runs, nothing shows the name: `member_names` decides "ended" at read time, and the
//!   directory lists only people with standing today.
//! - **An Article 17 erasure** removes both fields from every membership the account holds
//!   and marks them erased ([`erase_member_names`]). History then shows "Tidligere medlem",
//!   not even the role and year. It is the storage step of #3426's erasure flow, which
//!   decides who asks for it and what else goes. Database backups keep the old ciphertext
//!   until they age out; only deleting the FAU shreds it (key-service design §3.1).
//! - **Deleting the FAU** shreds its record key, and with it every name and address.

use fau_domain::time::Moment;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::error::MembershipError;
use super::profile::membership_ended;
use super::sql::{date_param, lock_tenant, ts_param, write_audit, Audit};

/// The daily sweep: clears the name and the contact address of every membership that has
/// ended, each clearing audited. Per FAU, under that FAU's lock, and re-checked under it, so
/// a role granted concurrently is never swept past. It clears at once, without the
/// account's three-month grace: that grace protects re-recognition of the account, and a
/// returning member states their name again when they accept. Returns how many memberships
/// it cleared.
pub async fn clear_ended_profiles(pool: &PgPool, at: Moment) -> Result<u64, MembershipError> {
    let today = date_param(at.today());
    let tenants: Vec<Uuid> = sqlx::query_scalar(&format!(
        "select distinct m.tenant_id from memberships m
          where (m.encrypted_display_name is not null or m.encrypted_contact_email is not null)
            and {}
          order by m.tenant_id",
        membership_ended("$1")
    ))
    .bind(&today)
    .fetch_all(pool)
    .await?;
    let mut cleared = 0;
    for tenant_id in tenants {
        let mut tx = pool.begin().await?;
        lock_tenant(&mut tx, tenant_id).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(&format!(
            "update memberships m
                set encrypted_display_name = null, encrypted_contact_email = null
              where m.tenant_id = $1
                and (m.encrypted_display_name is not null or m.encrypted_contact_email is not null)
                and {}
             returning m.id",
            membership_ended("$2")
        ))
        .bind(tenant_id)
        .bind(&today)
        .fetch_all(&mut *tx)
        .await?;
        for id in &ids {
            write_audit(
                &mut tx,
                at,
                Audit::system(
                    tenant_id,
                    "membership.profile_cleared",
                    "membership",
                    *id,
                    json!({ "cause": "membership_ended" }),
                ),
            )
            .await?;
        }
        tx.commit().await?;
        cleared += ids.len() as u64;
    }
    Ok(cleared)
}

/// Erases the account's name and contact address from every FAU it belongs to, in one
/// transaction, locking those FAU-er in id order, and marks each membership erased, an
/// ended one included: the marker is what makes history show "Tidligere medlem" instead of
/// the role and year. Idempotent: an already-erased membership is left as it is. Each
/// erasure is audited, without the name. Returns how many memberships it erased.
///
/// An erased membership cannot be accepted into again (`MembershipErased`): a new name on
/// the old row would re-attach history to it. #3426 decides how an erased person rejoins.
pub async fn erase_member_names(
    pool: &PgPool,
    account_id: Uuid,
    at: Moment,
) -> Result<u64, MembershipError> {
    let mut tx = pool.begin().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar(
        "select tenant_id from memberships where account_id = $1 order by tenant_id",
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut erased = 0;
    for tenant_id in tenants {
        lock_tenant(&mut tx, tenant_id).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "update memberships
                set name_erased_at = $3::timestamptz,
                    encrypted_display_name = null, encrypted_contact_email = null
              where tenant_id = $1 and account_id = $2 and name_erased_at is null
             returning id",
        )
        .bind(tenant_id)
        .bind(account_id)
        .bind(ts_param(at.now()))
        .fetch_all(&mut *tx)
        .await?;
        for id in &ids {
            write_audit(
                &mut tx,
                at,
                Audit::system(
                    tenant_id,
                    "membership.name_erased",
                    "membership",
                    *id,
                    json!({}),
                ),
            )
            .await?;
        }
        erased += ids.len() as u64;
    }
    tx.commit().await?;
    Ok(erased)
}
```

Create `backend/crates/persistence/src/membership/names.rs`:

```rust
//! Names for rendering history (groups design §4.2 as amended by Erik's D3, 28 September
//! 2026; #3502): an author, a minute-taker, an audit line. Every renderer reads them from
//! here, so the retention rules hold everywhere at once:
//! - an active membership has its name;
//! - an ended one (`membership_ended`, decided now, at read time, so a sweep that has not
//!   run yet never lets a name through) is shown as a role and its years, from the roles it
//!   actually held (`fau_domain::directory::history::role_label`);
//! - an Article 17 erasure is "Tidligere medlem", with neither name nor role.

use std::collections::HashMap;

use fau_crypto::Ciphertext;
use fau_domain::authz::Action;
use fau_domain::directory::history::HeldRole;
use fau_domain::membership::vocabulary::CapabilityClass;
use fau_domain::time::{oslo_today, Moment};
use sqlx::PgPool;
use uuid::Uuid;

use super::authz::{authorize, denied, read_transaction, Resource, Viewer};
use super::error::MembershipError;
use super::profile::membership_ended;
use super::sql::{date_param, from_micros, parse_date};

/// A membership as history shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberName {
    /// An active membership's name, encrypted under the record key with `DISPLAY_NAME_AAD`
    /// and the membership's id.
    Named(Ciphertext),
    /// An active membership created before migration 0008, which has no name.
    Unnamed,
    /// The membership has ended (D3): the roles it held, never empty, ordered by start. The
    /// renderer picks one for the event's date with `role_label` and renders it through the
    /// catalogue ("Leder 2025–2026").
    Ended(Vec<HeldRole>),
    /// An Article 17 erasure, or an ended membership that never held a role. Bokmål source
    /// string for the catalogue (#3439): "Tidligere medlem". Never listed by the directory.
    Former,
}

type MembershipRow = (Uuid, Option<Vec<u8>>, bool, bool);
type AssignmentRow = (Uuid, String, String, String, String, Option<i64>);

/// How `membership_ids` in the viewer's FAU appear in history (see [`MemberName`]). Ids not
/// in this FAU are left out, and a repeat comes back once. Members and admins only: a
/// guest's history rendering is decided with the first feature that shows a guest history
/// (#3503), so a guest gets `NotAuthorized`.
pub async fn member_names(
    pool: &PgPool,
    viewer: Viewer,
    membership_ids: &[Uuid],
    at: Moment,
) -> Result<Vec<(Uuid, MemberName)>, MembershipError> {
    let mut tx = read_transaction(pool).await?;
    authorize(&mut tx, viewer, Resource::Fau, Action::Read, at)
        .await?
        .map_err(|d| denied(d, MembershipError::NotAuthorized))?;
    let rows: Vec<MembershipRow> = sqlx::query_as(&format!(
        "select m.id, m.encrypted_display_name, m.name_erased_at is not null, {}
           from memberships m
          where m.tenant_id = $1 and m.id = any($2)",
        membership_ended("$3")
    ))
    .bind(viewer.tenant_id)
    .bind(membership_ids)
    .bind(date_param(at.today()))
    .fetch_all(&mut *tx)
    .await?;
    let ended: Vec<Uuid> = rows
        .iter()
        .filter(|(_, _, erased, ended)| !erased && *ended)
        .map(|(id, ..)| *id)
        .collect();
    let assignments: Vec<AssignmentRow> = sqlx::query_as(
        "select ra.membership_id, r.name, r.capability_class,
                to_char(ra.starts_on, 'YYYY-MM-DD'), to_char(ra.ends_on_exclusive, 'YYYY-MM-DD'),
                (extract(epoch from ra.revoked_at) * 1000000)::bigint
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
          where ra.tenant_id = $1 and ra.membership_id = any($2)
          order by ra.membership_id, ra.starts_on, ra.id",
    )
    .bind(viewer.tenant_id)
    .bind(&ended)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut held: HashMap<Uuid, Vec<HeldRole>> = HashMap::new();
    for (membership, name, class, starts, ends, revoked_us) in assignments {
        let from = parse_date(&starts)?;
        let mut until = parse_date(&ends)?;
        if let Some(us) = revoked_us {
            until = until.min(oslo_today(from_micros(us)?));
        }
        if from < until {
            held.entry(membership).or_default().push(HeldRole {
                name,
                class: CapabilityClass::from_code(&class).ok_or_else(MembershipError::decode)?,
                from,
                until,
            });
        }
    }

    let mut out: Vec<(Uuid, MemberName)> = Vec::with_capacity(rows.len());
    for id in membership_ids {
        if out.iter().any(|(m, _)| m == id) {
            continue;
        }
        let Some((_, name, erased, ended)) = rows.iter().find(|r| r.0 == *id) else {
            continue;
        };
        let shown = match (erased, ended) {
            (true, _) => MemberName::Former,
            (false, true) => match held.remove(id) {
                Some(roles) => MemberName::Ended(roles),
                None => MemberName::Former,
            },
            (false, false) => match name {
                Some(n) => MemberName::Named(Ciphertext::from_stored(n.clone())),
                None => MemberName::Unnamed,
            },
        };
        out.push((*id, shown));
    }
    Ok(out)
}
```

Apply to `backend/crates/persistence/src/membership/mod.rs`:

```diff
--- a/backend/crates/persistence/src/membership/mod.rs
+++ b/backend/crates/persistence/src/membership/mod.rs
@@ -31,8 +31,10 @@ mod events;
 mod groups;
 mod handover;
 mod invitations;
+mod names;
 mod profile;
 mod requests;
+mod retention;
 mod roles;
 mod signup;
 mod sql;
@@ -55,6 +57,7 @@ pub use invitations::{
     InvitationMessage, InvitationMessageView, IssueInvitation, IssuedInvitation, OfferedRole,
     RoleChoice, INVITATION_MESSAGE_AAD,
 };
+pub use names::{member_names, MemberName};
 pub use profile::{
     set_contact_email, set_display_name, MemberProfile, SetContactEmail, SetDisplayName,
     CONTACT_EMAIL_AAD, DISPLAY_NAME_AAD, MEMBER_FIELD_CIPHERTEXT_BYTES,
@@ -64,6 +67,7 @@ pub use requests::{
     decline_request, lapse_requests, AccessRequestMessage, CreateAccessRequest,
     CreateReplacementProposal, RequestDecision, ACCESS_REQUEST_MESSAGE_AAD, MESSAGE_MAX_BYTES,
 };
+pub use retention::{clear_ended_profiles, erase_member_names};
 pub use roles::{
     grant_role, revoke_membership, revoke_role_assignment, GrantRole, RevokeAssignment,
     RevokeMembership,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test directory_retention && cargo test -p fau-domain --lib -- directory`
Expected: PASS. `directory_retention` shows 3 passed. The domain's `directory` tests are 28: Tasks 2 and 3's 22, and 6 in `history`.

Mutation checks, each run in the dry run:
- Drop `and ra.ends_on_exclusive > {date_param}::date` from `membership_ended`, and the sweep and erasure tests fail: a role that ran out counts as running.
- Drop `and ra.revoked_at is null`, and the stepped-down member keeps their fields.
- Make `member_names` ignore "ended" (`match (erased, *ended && false)`), and the read before the sweep returns the name.
- In `history.rs`, rank the admin class last, and `on_one_day_an_admin_role_wins_then_the_earliest_start` fails. Pick the earliest end instead of the latest after every role has ended, and `outside_every_role_the_nearest_end_or_the_first_start_is_shown` fails.
- Drop the `authorize` call from `member_names`, and the guest row returns names.

`ra.tenant_id = m.tenant_id` in `membership_ended` is the composite-key convention, not a tested guard: membership ids are UUIDv7 and unique across FAU-er, so removing it changes no result.

- [ ] **Step 5: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/domain/src/directory/history.rs \
        backend/crates/domain/src/directory/mod.rs \
        backend/crates/persistence/src/membership/retention.rs \
        backend/crates/persistence/src/membership/names.rs \
        backend/crates/persistence/src/membership/mod.rs \
        backend/crates/app/tests/directory_retention.rs
git commit -m "Clear a name and contact address when the membership ends; show role and year in history (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: The directory read, through the group rule (acceptance: guests see only their own groups)

**Files:**
- Modify: `backend/crates/persistence/src/membership/groups.rs` (share `readable_groups` and `group_members_sql`)
- Create: `backend/crates/persistence/src/membership/directory.rs`
- Modify: `backend/crates/persistence/src/membership/mod.rs`
- Modify: `backend/crates/app/tests/authorization.rs` (directory entries in the matrix)
- Test: `backend/crates/app/tests/directory.rs`

**Interfaces:**
- Consumes:
  - `membership_access`, `decide(.., Target::Fau, Action::Read)`, `read_transaction`, `ASSIGNMENT_VALID_ON_3` and `USABLE_ACCOUNT`;
  - `MemberName` (Task 6);
  - `SectionId`, `RoleHeld` and `Place` (Task 3);
  - the `world()` fixture from #3501.
- Produces:
  - `pub(crate) readable_groups(conn, Viewer, Capability, Moment) -> Result<Vec<GroupView>, _>`, which `list_groups` now uses;
  - `pub(crate) group_members_sql(group_filter: &str) -> String`, returning `(group_id, membership_id, by_hand, through_role)` with `$1` tenant and `$3` date, which `list_group_members` now uses;
  - `enum DirectoryAddress { Contact(Ciphertext), Login(Email) }`;
  - `struct DirectoryPerson { membership_id, is_viewer, name: MemberName, address, is_guest, roles: Vec<RoleHeld>, group_ids: Vec<Uuid> }`;
  - `struct DirectorySection { section: SectionId, encrypted_group_name: Option<Ciphertext>, members: Vec<Uuid> }`;
  - `struct DirectoryRead { sections, people }`;
  - `pub(crate) load(conn, Viewer, Moment) -> Result<DirectoryRead, _>`;
  - `member_directory(pool, Viewer, Moment) -> Result<DirectoryRead, MembershipError>`.

- [ ] **Step 1: Write the failing tests**

The matrix rows (spec §10: "directory entry" is one of the matrix's resource types):

Apply to `backend/crates/app/tests/authorization.rs`:

```diff
--- a/backend/crates/app/tests/authorization.rs
+++ b/backend/crates/app/tests/authorization.rs
@@ -7,8 +7,12 @@
 use common::membership::*;
 use common::TestDb;
 use fau_domain::authz::{Action, Decision, Denied};
+use fau_domain::directory::listing::SectionId;
 use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
-use fau_persistence::membership::{authorize, grant_role, GrantRole, Resource, RoleChoice, Viewer};
+use fau_persistence::membership::{
+    authorize, grant_role, member_directory, GrantRole, MembershipError, Resource, RoleChoice,
+    Viewer,
+};
 use sqlx::PgPool;
 use uuid::Uuid;
 
@@ -512,3 +516,107 @@
     .await;
     assert_eq!(check(&pool, v, Resource::Fau, Action::Read, T0).await, N);
 }
+
+/// Directory entries (§10 lists "directory entry" among the matrix's resource types; #3502).
+/// An entry's audience is its person's memberships (§3.2): the FAU-wide section, which only
+/// members and admins see and which never lists a guest, and each group the viewer may
+/// read. (viewer, sections seen, people seen); `none_*` are refused outright.
+#[rustfmt::skip]
+const DIRECTORY: &[(&str, &[&str], &[&str])] = &[
+    ("admin_in",   &["fau", "open", "closed", "other"],
+                   &["admin_in", "admin_out", "member_in", "member_out", "guest_in", "guest_out"]),
+    ("admin_out",  &["fau", "open", "closed", "other"],
+                   &["admin_in", "admin_out", "member_in", "member_out", "guest_in", "guest_out"]),
+    ("member_in",  &["fau", "open", "closed"],
+                   &["admin_in", "admin_out", "member_in", "member_out", "guest_in"]),
+    ("member_out", &["fau", "open"],
+                   &["admin_in", "admin_out", "member_in", "member_out", "guest_in"]),
+    ("guest_in",   &["open", "closed"], &["admin_in", "member_in", "guest_in"]),
+    ("guest_out",  &["other"],          &["guest_out"]),
+    ("none_in",    &[], &[]),
+    ("none_out",   &[], &[]),
+];
+
+/// What each section lists, whoever reads it: the same people for every viewer allowed to
+/// see the section. `none_in` was added to `open` and `closed` by hand but has no standing.
+#[rustfmt::skip]
+const SECTION_MEMBERS: &[(&str, &[&str])] = &[
+    ("fau",    &["admin_in", "admin_out", "member_in", "member_out"]),
+    ("open",   &["admin_in", "member_in", "guest_in"]),
+    ("closed", &["admin_in", "member_in", "guest_in"]),
+    ("other",  &["guest_out"]),
+];
+
+#[tokio::test]
+async fn directory_entries_in_the_matrix() {
+    let db = TestDb::migrated().await;
+    let pool = db.app_pool().await;
+    let w = world(&pool).await;
+    let section_name = |s: SectionId| match s {
+        SectionId::Fau => "fau",
+        SectionId::Group(g) if g == w.open => "open",
+        SectionId::Group(g) if g == w.closed => "closed",
+        SectionId::Group(g) if g == w.other => "other",
+        SectionId::Group(_) => "unexpected",
+    };
+    let person_name = |m: Uuid| {
+        VIEWERS
+            .iter()
+            .find(|n| w.membership(n) == m)
+            .copied()
+            .unwrap_or("unexpected")
+    };
+    let sorted = |mut v: Vec<&'static str>| {
+        v.sort_unstable();
+        v
+    };
+
+    let mut viewers: Vec<&str> = DIRECTORY.iter().map(|row| row.0).collect();
+    viewers.sort_unstable();
+    let mut all = VIEWERS.to_vec();
+    all.sort_unstable();
+    assert_eq!(viewers, all, "every viewer has a row");
+
+    let mut failures = Vec::new();
+    for &(viewer, sections, people) in DIRECTORY {
+        let read = member_directory(&pool, w.viewer(viewer), at(T0)).await;
+        if viewer.starts_with("none") {
+            if read != Err(MembershipError::NotAuthorized) {
+                failures.push(format!("{viewer}: want NotAuthorized, got {read:?}"));
+            }
+            continue;
+        }
+        let read = read.unwrap();
+        let got_sections = sorted(
+            read.sections
+                .iter()
+                .map(|s| section_name(s.section))
+                .collect(),
+        );
+        if got_sections != sorted(sections.to_vec()) {
+            failures.push(format!(
+                "{viewer} sections: want {sections:?}, got {got_sections:?}"
+            ));
+        }
+        let got_people = sorted(
+            read.people
+                .iter()
+                .map(|p| person_name(p.membership_id))
+                .collect(),
+        );
+        if got_people != sorted(people.to_vec()) {
+            failures.push(format!(
+                "{viewer} people: want {people:?}, got {got_people:?}"
+            ));
+        }
+        for s in &read.sections {
+            let name = section_name(s.section);
+            let want = SECTION_MEMBERS.iter().find(|(n, _)| *n == name).unwrap().1;
+            let got = sorted(s.members.iter().map(|m| person_name(*m)).collect());
+            if got != sorted(want.to_vec()) {
+                failures.push(format!("{viewer} sees {name} as {got:?}, want {want:?}"));
+            }
+        }
+    }
+    assert!(failures.is_empty(), "{failures:#?}");
+}
```

Create `backend/crates/app/tests/directory.rs`:

```rust
//! The member directory's reads (groups design §4.3, §4.4; #3502): its sections agree with
//! the group reads, an entry shows what a person represents, and only people with standing
//! today are listed. The per-viewer matrix is in authorization.rs.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::directory::listing::{Place, RoleHeld, SectionId};
use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

fn year() -> Period {
    period(day(2026, 9, 1), day(2027, 9, 1))
}

fn sorted(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort_unstable();
    v
}

/// Mutation check: give the directory its own copy of the member SQL, or let it skip
/// `decide` for groups, and one of these comparisons fails for some viewer.
#[tokio::test]
async fn sections_are_exactly_the_readable_groups_with_their_member_lists() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let t0 = at(T0);
    for name in VIEWERS.iter().filter(|n| !n.starts_with("none")) {
        let viewer = w.viewer(name);
        let read = member_directory(&pool, viewer, t0).await.unwrap();
        let groups: Vec<Uuid> = read
            .sections
            .iter()
            .filter_map(|s| match s.section {
                SectionId::Group(g) => Some(g),
                SectionId::Fau => None,
            })
            .collect();
        let listed: Vec<Uuid> = list_groups(&pool, viewer, t0)
            .await
            .unwrap()
            .iter()
            .map(|g| g.group_id)
            .collect();
        assert_eq!(sorted(groups.clone()), sorted(listed), "{name}");
        for g in groups {
            let section = read
                .sections
                .iter()
                .find(|s| s.section == SectionId::Group(g))
                .unwrap();
            let members: Vec<Uuid> = list_group_members(&pool, viewer, g, t0)
                .await
                .unwrap()
                .iter()
                .map(|m| m.membership_id)
                .collect();
            assert_eq!(sorted(section.members.clone()), sorted(members), "{name}");
            assert!(section.encrypted_group_name.is_some());
        }
    }
}

async fn named_role(
    pool: &PgPool,
    tenant: Uuid,
    name: &str,
    unit: Option<Uuid>,
    cohort: Option<Uuid>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class, unit_id, cohort_id)
         values ($1, $2, $3, 'member', $4, $5)",
    )
    .bind(tenant)
    .bind(id)
    .bind(name)
    .bind(unit)
    .bind(cohort)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn plaintext_name(pool: &PgPool, table: &str, id: Uuid) -> String {
    sqlx::query_scalar(&format!("select name from {table} where id = $1"))
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_entry_shows_what_the_person_represents_and_which_address_applies() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let unit = seed_unit(&pool, fau.tenant_id).await;
    let cohort = seed_cohort(&pool, fau.tenant_id).await;
    let on_unit = named_role(&pool, fau.tenant_id, "Kontaktforelder", Some(unit), None).await;
    let on_cohort = named_role(&pool, fau.tenant_id, "Trinnkontakt", None, Some(cohort)).await;
    let group = seed_bound_group(&pool, &fau, Visibility::Open, Some(unit), None).await;

    let kari = add_member(
        &pool,
        &fau,
        "kari@example.test",
        RoleChoice::Existing(on_unit),
        year(),
        t0,
    )
    .await
    .membership_id;
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: kari,
            role: RoleChoice::Existing(on_cohort),
            period: year(),
        },
        t0,
    )
    .await
    .unwrap();
    // A past role is not shown: the directory shows the present.
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: kari,
            role: new_role("Tidligere kasserer", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2026, 9, 24)),
        },
        t0,
    )
    .await
    .unwrap();
    let contact = {
        let mut v = vec![1u8, 77];
        v.extend_from_slice(&[0u8; 40]);
        Ciphertext::from_stored(v)
    };
    set_contact_email(
        &pool,
        SetContactEmail {
            tenant_id: fau.tenant_id,
            membership_id: kari,
            encrypted_contact_email: Some(contact.clone()),
        },
        t0,
    )
    .await
    .unwrap();
    let guest = add_guest(&pool, &fau, "gjest@example.test", group, year(), t0)
        .await
        .membership_id;

    let later = at("2026-09-25T10:00:00Z");
    let read = member_directory(
        &pool,
        Viewer {
            tenant_id: fau.tenant_id,
            membership_id: kari,
        },
        later,
    )
    .await
    .unwrap();
    let entry = |m: Uuid| read.people.iter().find(|p| p.membership_id == m).unwrap();

    let k = entry(kari);
    assert!(k.is_viewer && !k.is_guest);
    assert_eq!(k.address, DirectoryAddress::Contact(contact));
    assert_eq!(k.name, MemberName::Named(placeholder_envelope()));
    let mut roles = k.roles.clone();
    roles.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        roles,
        [
            RoleHeld {
                name: "Kontaktforelder".into(),
                place: Some(Place::Unit(
                    plaintext_name(&pool, "organization_units", unit).await
                )),
            },
            RoleHeld {
                name: "Trinnkontakt".into(),
                place: Some(Place::Cohort(
                    plaintext_name(&pool, "cohorts", cohort).await
                )),
            },
        ]
    );
    assert_eq!(k.group_ids, [group], "in the unit's group through the role");

    let g = entry(guest);
    assert!(g.is_guest && !g.is_viewer);
    assert_eq!(
        g.address,
        DirectoryAddress::Login(email("gjest@example.test"))
    );
    assert_eq!(g.group_ids, [group]);
    let fau_section = read
        .sections
        .iter()
        .find(|s| s.section == SectionId::Fau)
        .unwrap();
    assert!(
        !fau_section.members.contains(&guest),
        "never in the FAU-wide section"
    );
    assert!(fau_section.members.contains(&kari));
}

#[tokio::test]
async fn only_people_with_standing_today_are_listed() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let current = join(&pool, &fau, "current@example.test", year()).await;
    let future = join(
        &pool,
        &fau,
        "future@example.test",
        period(day(2026, 10, 1), day(2027, 9, 1)),
    )
    .await;
    let ended = join(
        &pool,
        &fau,
        "ended@example.test",
        period(day(2026, 9, 1), day(2026, 9, 24)),
    )
    .await;
    let disabled = join(&pool, &fau, "disabled@example.test", year()).await;
    let revoked = join(&pool, &fau, "revoked@example.test", year()).await;
    let erased = join(&pool, &fau, "erased@example.test", year()).await;
    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(disabled.account_id)
        .execute(&pool)
        .await
        .unwrap();
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: revoked.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    erase_member_names(&pool, erased.account_id, t0)
        .await
        .unwrap();

    let viewer = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: current.membership_id,
    };
    let later = at("2026-09-25T10:00:00Z");
    let listed: Vec<Uuid> = member_directory(&pool, viewer, later)
        .await
        .unwrap()
        .people
        .iter()
        .map(|p| p.membership_id)
        .collect();
    assert_eq!(
        sorted(listed),
        sorted(vec![fau.admin_membership_id, current.membership_id])
    );
    // An erased membership still has its roles, so it is not listed but may still read.
    // #3426's erasure flow decides whether the membership ends with it.
    for absent in [future, ended, disabled, revoked] {
        let refused = member_directory(
            &pool,
            Viewer {
                tenant_id: fau.tenant_id,
                membership_id: absent.membership_id,
            },
            later,
        )
        .await;
        assert_eq!(refused, Err(MembershipError::NotAuthorized));
    }
}

async fn join(pool: &PgPool, fau: &Fau, address: &str, p: Period) -> Accepted {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        p,
        at(T0),
    )
    .await
}

#[tokio::test]
async fn a_viewer_from_another_fau_is_refused_and_sees_nothing_of_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    // B's admin, naming A's tenant: no standing there.
    let crossed = Viewer {
        tenant_id: a.tenant_id,
        membership_id: b.admin_membership_id,
    };
    assert_eq!(
        member_directory(&pool, crossed, t0).await,
        Err(MembershipError::NotAuthorized)
    );
    // Positive control: B's admin sees only B.
    let own = member_directory(
        &pool,
        Viewer {
            tenant_id: b.tenant_id,
            membership_id: b.admin_membership_id,
        },
        t0,
    )
    .await
    .unwrap();
    let ids: Vec<Uuid> = own.people.iter().map(|p| p.membership_id).collect();
    assert_eq!(ids, [b.admin_membership_id]);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p fau-app --test directory --test authorization`
Expected: compile errors, `cannot find function member_directory` and `cannot find type DirectoryAddress`.

- [ ] **Step 3: Share the group reads**

This refactor changes no behaviour. `group_reads.rs` must stay green on its own before Step 4.

Apply to `backend/crates/persistence/src/membership/groups.rs`:

```diff
--- a/backend/crates/persistence/src/membership/groups.rs
+++ b/backend/crates/persistence/src/membership/groups.rs
@@ -654,14 +654,26 @@
     if access.capability == Capability::None {
         return Err(MembershipError::NotAuthorized);
     }
+    let visible = readable_groups(&mut tx, viewer, access.capability, at).await?;
+    tx.commit().await?;
+    Ok(visible)
+}
+
+/// Every group `capability` may read, with its facts, on the caller's snapshot: the one
+/// source of [`list_groups`] and the member directory's group sections (#3502).
+pub(crate) async fn readable_groups(
+    conn: &mut PgConnection,
+    viewer: Viewer,
+    capability: Capability,
+    at: Moment,
+) -> Result<Vec<GroupView>, MembershipError> {
     let sql = format!("{} order by g.id", group_select());
     let rows: Vec<GroupRow> = sqlx::query_as(&sql)
         .bind(viewer.tenant_id)
         .bind(viewer.membership_id)
         .bind(date_param(at.today()))
-        .fetch_all(&mut *tx)
+        .fetch_all(&mut *conn)
         .await?;
-    tx.commit().await?;
     let mut visible = Vec::new();
     for row in rows {
         let group = view(row)?;
@@ -670,7 +682,7 @@
             archived: group.archived,
             viewer_in_group: group.viewer_in_group,
         };
-        if decide(access.capability, Target::Group(facts), Action::Read).is_ok() {
+        if decide(capability, Target::Group(facts), Action::Read).is_ok() {
             visible.push(group);
         }
     }
@@ -717,27 +729,7 @@
     authorize(&mut tx, viewer, Resource::Group(group_id), Action::Read, at)
         .await?
         .map_err(|d| denied(d, MembershipError::UnknownGroup))?;
-    let sql = format!(
-        "select s.membership_id, bool_or(s.by_hand), bool_or(not s.by_hand)
-           from (select gm.membership_id, true as by_hand
-                   from group_members gm
-                  where gm.tenant_id = $1 and gm.group_id = $2 and gm.removed_at is null
-                 union all
-                 select ra.membership_id, false
-                   from groups g
-                   join roles r             on r.tenant_id = g.tenant_id and {ROLE_FOLLOWS_GROUP}
-                   join role_assignments ra on ra.tenant_id = r.tenant_id and ra.role_id = r.id
-                  where g.tenant_id = $1 and g.id = $2 and {ASSIGNMENT_VALID_ON_3}) s
-           join memberships m on m.tenant_id = $1 and m.id = s.membership_id
-           join accounts a    on a.id = m.account_id
-          where {USABLE_ACCOUNT}
-            and exists (select 1 from role_assignments ra
-                         where ra.tenant_id = m.tenant_id and ra.membership_id = m.id
-                           and {ASSIGNMENT_VALID_ON_3})
-          group by s.membership_id
-          order by s.membership_id"
-    );
-    let rows: Vec<(Uuid, bool, bool)> = sqlx::query_as(&sql)
+    let rows: Vec<(Uuid, Uuid, bool, bool)> = sqlx::query_as(&group_members_sql("= $2"))
         .bind(viewer.tenant_id)
         .bind(group_id)
         .bind(date_param(at.today()))
@@ -747,7 +739,7 @@
     Ok(rows
         .into_iter()
         .map(
-            |(membership_id, added_by_hand, through_role)| GroupMemberView {
+            |(_, membership_id, added_by_hand, through_role)| GroupMemberView {
                 membership_id,
                 added_by_hand,
                 through_role,
@@ -755,3 +747,33 @@
         )
         .collect())
 }
+
+/// `(group_id, membership_id, added_by_hand, through_role)` for the current members of the
+/// groups `g.id <group_filter>` (for example `= $2` or `= any($2)`), with `$1` the tenant and
+/// `$3` the date: added by hand and not removed, or holding a role valid on `$3` that the
+/// group follows. Only people with standing that day count -- a usable membership with some
+/// role assignment valid then; a handover grant is a temporary admin-class recovery right
+/// (§6.2), not group membership. Ordered by group, then membership. The one source of
+/// [`list_group_members`] and the member directory's group sections (#3502).
+pub(crate) fn group_members_sql(group_filter: &str) -> String {
+    format!(
+        "select s.group_id, s.membership_id, bool_or(s.by_hand), bool_or(not s.by_hand)
+           from (select gm.group_id, gm.membership_id, true as by_hand
+                   from group_members gm
+                  where gm.tenant_id = $1 and gm.group_id {group_filter} and gm.removed_at is null
+                 union all
+                 select g.id, ra.membership_id, false
+                   from groups g
+                   join roles r             on r.tenant_id = g.tenant_id and {ROLE_FOLLOWS_GROUP}
+                   join role_assignments ra on ra.tenant_id = r.tenant_id and ra.role_id = r.id
+                  where g.tenant_id = $1 and g.id {group_filter} and {ASSIGNMENT_VALID_ON_3}) s
+           join memberships m on m.tenant_id = $1 and m.id = s.membership_id
+           join accounts a    on a.id = m.account_id
+          where {USABLE_ACCOUNT}
+            and exists (select 1 from role_assignments ra
+                         where ra.tenant_id = m.tenant_id and ra.membership_id = m.id
+                           and {ASSIGNMENT_VALID_ON_3})
+          group by s.group_id, s.membership_id
+          order by s.group_id, s.membership_id"
+    )
+}
```

Run: `cargo test -p fau-app --test group_reads --test groups`
Expected: PASS, unchanged.

- [ ] **Step 4: Write the directory read**

Create `backend/crates/persistence/src/membership/directory.rs`:

```rust
//! The member directory's read (groups design §4; #3502): who a viewer may see and what
//! each person represents. `export.rs` hands over a selection's addresses.
//!
//! **What a viewer sees** (§4.4, §3.2, §3.3):
//! - the FAU-wide section, every current member and admin, for members and admins only --
//!   an FAU-wide audience never includes guests, as viewers or as entries;
//! - one section per group the viewer may read, through the same facts and the same rule as
//!   `list_groups` (`readable_groups`), listing its current members as `list_group_members`
//!   does (`group_members_sql`), guests included;
//! - so a guest sees exactly the members of their own groups, and members see a guest only
//!   inside a group they can read.
//!
//! "Current" means standing today: a usable membership holding a role assignment valid
//! today. A person whose name an Article 17 erasure removed is not listed. Everyone listed
//! is therefore active, so under D3 (28 September 2026) their name still applies: an entry's
//! `name` is only ever `MemberName::Named` or `MemberName::Unnamed`, never `Ended` or
//! `Former`.
//!
//! **What a person represents** is derived, never typed: their role assignments valid
//! today (role name, plus unit or cohort), and the listed groups they are in (§4.1).
//!
//! Every read runs in one snapshot with the authorization it depends on. Names and contact
//! addresses leave this module as ciphertext; the session decrypts them under the FAU's
//! record key and orders the result with `fau_domain::directory::listing::arrange`.

use std::collections::{BTreeMap, HashSet};

use fau_crypto::Ciphertext;
use fau_domain::authz::{decide, Action, Target};
use fau_domain::directory::listing::{Place, RoleHeld, SectionId};
use fau_domain::email::Email;
use fau_domain::membership::access::Capability;
use fau_domain::time::Moment;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::access::membership_access;
use super::authz::{read_transaction, Viewer, ASSIGNMENT_VALID_ON_3};
use super::error::MembershipError;
use super::groups::{group_members_sql, readable_groups};
use super::names::MemberName;
use super::sql::{date_param, USABLE_ACCOUNT};

/// The address the directory shows for a person (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryAddress {
    /// The member's own contact address, encrypted with `CONTACT_EMAIL_AAD`.
    Contact(Ciphertext),
    /// No contact address is set, so the login address is shown.
    Login(Email),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPerson {
    pub membership_id: Uuid,
    /// The viewer's own entry, where the screen offers editing.
    pub is_viewer: bool,
    /// `Named`, or `Unnamed` for a membership created before migration 0008.
    pub name: MemberName,
    pub address: DirectoryAddress,
    /// Holds only guest-class roles today: marked "Gjest".
    pub is_guest: bool,
    /// What their role assignments valid today say they represent. Plaintext school
    /// structure; the session orders them by name.
    pub roles: Vec<RoleHeld>,
    /// The listed groups this person is in, by group id.
    pub group_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectorySection {
    pub section: SectionId,
    /// The group's name under the record key (`GROUP_NAME_AAD`); `None` for the FAU-wide
    /// section.
    pub encrypted_group_name: Option<Ciphertext>,
    /// Ordered by membership id; the session orders by name after decrypting.
    pub members: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryRead {
    /// The FAU-wide section first when the viewer may see it, then groups by id.
    pub sections: Vec<DirectorySection>,
    /// Everyone listed in any section, once, by membership id.
    pub people: Vec<DirectoryPerson>,
}

type PersonRow = (Uuid, bool, Option<Vec<u8>>, Option<Vec<u8>>, String, bool);

/// Loads the directory for `viewer` on the caller's snapshot. `member_directory` and
/// `export_addresses` both read through it, so an export can only ever reach people the
/// directory would show.
pub(crate) async fn load(
    conn: &mut PgConnection,
    viewer: Viewer,
    at: Moment,
) -> Result<DirectoryRead, MembershipError> {
    let access =
        membership_access(conn, viewer.tenant_id, viewer.membership_id, at.today()).await?;
    if access.capability == Capability::None {
        return Err(MembershipError::NotAuthorized);
    }
    let today = date_param(at.today());

    // Everyone with standing today, and whether they hold anything but guest roles.
    let sql = format!(
        "select m.id, m.id = $2, m.encrypted_display_name, m.encrypted_contact_email, a.email,
                not exists (select 1 from role_assignments ra
                              join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
                             where ra.tenant_id = m.tenant_id and ra.membership_id = m.id
                               and {ASSIGNMENT_VALID_ON_3} and r.capability_class <> 'guest')
           from memberships m
           join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and {USABLE_ACCOUNT} and m.name_erased_at is null
            and exists (select 1 from role_assignments ra
                         where ra.tenant_id = m.tenant_id and ra.membership_id = m.id
                           and {ASSIGNMENT_VALID_ON_3})
          order by m.id"
    );
    let rows: Vec<PersonRow> = sqlx::query_as(&sql)
        .bind(viewer.tenant_id)
        .bind(viewer.membership_id)
        .bind(&today)
        .fetch_all(&mut *conn)
        .await?;
    let mut standing: BTreeMap<Uuid, DirectoryPerson> = BTreeMap::new();
    for (id, is_viewer, name, contact, login, is_guest) in rows {
        let address = match contact {
            Some(ct) => DirectoryAddress::Contact(Ciphertext::from_stored(ct)),
            None => DirectoryAddress::Login(
                Email::parse(&login).map_err(|_| MembershipError::decode())?,
            ),
        };
        standing.insert(
            id,
            DirectoryPerson {
                membership_id: id,
                is_viewer,
                name: name
                    .map(|n| MemberName::Named(Ciphertext::from_stored(n)))
                    .unwrap_or(MemberName::Unnamed),
                address,
                is_guest,
                roles: Vec::new(),
                group_ids: Vec::new(),
            },
        );
    }

    let mut sections = Vec::new();
    if decide(access.capability, Target::Fau, Action::Read).is_ok() {
        sections.push(DirectorySection {
            section: SectionId::Fau,
            encrypted_group_name: None,
            members: standing
                .values()
                .filter(|p| !p.is_guest)
                .map(|p| p.membership_id)
                .collect(),
        });
    }
    let groups = readable_groups(conn, viewer, access.capability, at).await?;
    let group_ids: Vec<Uuid> = groups.iter().map(|g| g.group_id).collect();
    let rows: Vec<(Uuid, Uuid, bool, bool)> = sqlx::query_as(&group_members_sql("= any($2)"))
        .bind(viewer.tenant_id)
        .bind(&group_ids)
        .bind(&today)
        .fetch_all(&mut *conn)
        .await?;
    for g in groups {
        sections.push(DirectorySection {
            section: SectionId::Group(g.group_id),
            encrypted_group_name: Some(g.encrypted_name),
            members: rows
                .iter()
                .filter(|(group, m, _, _)| *group == g.group_id && standing.contains_key(m))
                .map(|(_, m, _, _)| *m)
                .collect(),
        });
    }

    let listed: HashSet<Uuid> = sections
        .iter()
        .flat_map(|s| s.members.iter().copied())
        .collect();
    standing.retain(|id, _| listed.contains(id));
    for s in &sections {
        if let SectionId::Group(g) = s.section {
            for m in &s.members {
                if let Some(p) = standing.get_mut(m) {
                    p.group_ids.push(g);
                }
            }
        }
    }

    // What each listed person represents: role assignments valid today.
    let ids: Vec<Uuid> = standing.keys().copied().collect();
    let roles: Vec<(Uuid, String, Option<String>, Option<String>)> = sqlx::query_as(&format!(
        "select ra.membership_id, r.name, u.name, c.name
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
           left join organization_units u on u.tenant_id = r.tenant_id and u.id = r.unit_id
           left join cohorts c            on c.tenant_id = r.tenant_id and c.id = r.cohort_id
          where ra.tenant_id = $1 and ra.membership_id = any($2) and {ASSIGNMENT_VALID_ON_3}
          order by ra.membership_id, ra.id"
    ))
    .bind(viewer.tenant_id)
    .bind(&ids)
    .bind(&today)
    .fetch_all(&mut *conn)
    .await?;
    for (m, name, unit, cohort) in roles {
        let place = match (unit, cohort) {
            (Some(u), _) => Some(Place::Unit(u)),
            (None, Some(c)) => Some(Place::Cohort(c)),
            (None, None) => None,
        };
        if let Some(p) = standing.get_mut(&m) {
            p.roles.push(RoleHeld { name, place });
        }
    }

    Ok(DirectoryRead {
        sections,
        people: standing.into_values().collect(),
    })
}

/// The directory as `viewer` may see it today (§4.3, §4.4). A viewer with no standing gets
/// `NotAuthorized`.
pub async fn member_directory(
    pool: &PgPool,
    viewer: Viewer,
    at: Moment,
) -> Result<DirectoryRead, MembershipError> {
    let mut tx = read_transaction(pool).await?;
    let read = load(&mut tx, viewer, at).await?;
    tx.commit().await?;
    Ok(read)
}
```

Apply to `backend/crates/persistence/src/membership/mod.rs`:

```diff
--- a/backend/crates/persistence/src/membership/mod.rs
+++ b/backend/crates/persistence/src/membership/mod.rs
@@ -26,6 +26,7 @@
 
 mod access;
 mod authz;
+mod directory;
 mod error;
 mod events;
 mod groups;
@@ -42,6 +43,9 @@
 
 pub use access::effective_access;
 pub use authz::{authorize, read_transaction, Resource, Viewer};
+pub use directory::{
+    member_directory, DirectoryAddress, DirectoryPerson, DirectoryRead, DirectorySection,
+};
 pub use error::{ExistingFau, MembershipError};
 pub use events::{Change, Hub, HubClock, Subscription, EVENTS_CHANNEL, SUBSCRIPTION_BUFFER};
 pub use groups::{
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test directory --test authorization --test group_reads`
Expected: PASS: `directory` 4, `authorization` 7 (including `directory_entries_in_the_matrix`) and `group_reads` 4.

Mutation checks, run while planning, each failing `directory_entries_in_the_matrix` or `only_people_with_standing_today_are_listed`:
- make the FAU-wide section unconditional (`if true`);
- let guests into it (drop `!p.is_guest`);
- drop `m.name_erased_at is null`.

- [ ] **Step 6: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/persistence/src/membership/groups.rs \
        backend/crates/persistence/src/membership/directory.rs \
        backend/crates/persistence/src/membership/mod.rs \
        backend/crates/app/tests/authorization.rs \
        backend/crates/app/tests/directory.rs
git commit -m "Read the member directory through the group authorization rule (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The audited export of addresses (acceptance: the audit holds a count, never addresses)

**Files:**
- Create: `backend/crates/persistence/src/membership/export.rs`
- Modify: `backend/crates/persistence/src/membership/error.rs`, `mod.rs`
- Test: `backend/crates/app/tests/address_export.rs`

**Interfaces:**
- Consumes: `directory::load`, `DirectoryRead`, `DirectoryAddress` (Task 7); `write_audit` and `Audit::member`.
- Produces:
  - `enum ExportPurpose { Mailto, Copy }` and `enum ExportScope { All, Fau, Group(Uuid) }`;
  - `struct AddressExport { purpose, scope, membership_ids: Vec<Uuid> }` and `struct ExportedRecipient { membership_id, address: DirectoryAddress }`;
  - `export_addresses(pool, Viewer, AddressExport, Moment) -> Result<Vec<ExportedRecipient>, MembershipError>`;
  - `MembershipError::EmptySelection`;
  - audit action `directory.addresses_exported {purpose, recipient_count, scope, group_id}`.

- [ ] **Step 1: Write the failing test**

Create `backend/crates/app/tests/address_export.rs`:

```rust
//! Handing over a selection's addresses for "Skriv e-post" or "Kopier adresser" (groups
//! design §4.3, §4.4, §10; #3502): each person once, only people the viewer's scope lists,
//! and an audit entry that holds the count and the scope, never the addresses.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

async fn exported(pool: &PgPool) -> i64 {
    count(
        pool,
        "select count(*) from audit_events where action = 'directory.addresses_exported'",
    )
    .await
}

async fn last_params(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "select params::text from audit_events where action = 'directory.addresses_exported'
          order by occurred_at desc, id desc limit 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

fn export(purpose: ExportPurpose, scope: ExportScope, ids: &[Uuid]) -> AddressExport {
    AddressExport {
        purpose,
        scope,
        membership_ids: ids.to_vec(),
    }
}

#[tokio::test]
async fn each_person_comes_back_once_and_the_audit_holds_only_the_count_and_the_scope() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let (admin, member, guest) = (
        w.membership("admin_in"),
        w.membership("member_in"),
        w.membership("guest_in"),
    );

    // Admin ticked in two groups: once, in first-selected order.
    let got = export_addresses(
        &pool,
        w.viewer("member_out"),
        export(
            ExportPurpose::Mailto,
            ExportScope::All,
            &[admin, member, admin, guest],
        ),
        at(T0),
    )
    .await
    .unwrap();
    let ids: Vec<Uuid> = got.iter().map(|r| r.membership_id).collect();
    assert_eq!(ids, [admin, member, guest]);
    assert_eq!(
        got[2].address,
        DirectoryAddress::Login(email("guest-in@example.test")),
        "a member sees a guest's address inside a group they can read"
    );

    let params = last_params(&pool).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&params).unwrap(),
        serde_json::json!({
            "purpose": "mailto", "recipient_count": 3, "scope": "all", "group_id": null
        })
    );
    assert!(!params.contains('@'), "no address");
    for id in [admin, member, guest] {
        assert!(!params.contains(&id.to_string()), "nobody's id");
    }

    export_addresses(
        &pool,
        w.viewer("member_out"),
        export(ExportPurpose::Copy, ExportScope::Group(w.open), &[guest]),
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&last_params(&pool).await).unwrap(),
        serde_json::json!({
            "purpose": "copy", "recipient_count": 1, "scope": "group", "group_id": w.open
        })
    );
    assert_eq!(exported(&pool).await, 2);
}

/// Every refusal writes nothing, and comes in a fixed order, so no answer depends on
/// whether a hidden id exists. Mutation check: skip the scope check and the `member_out` /
/// `closed` row passes; check memberships before the scope and the unknown-group rows
/// answer `UnknownMembership`.
#[tokio::test]
async fn a_selection_outside_what_the_viewer_sees_is_refused_and_writes_nothing() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let other_fau = active_fau(&pool, "admin-b@example.test", at(T0)).await;
    let m = |n: &str| w.membership(n);
    let unknown = Uuid::now_v7();
    let rows: Vec<(&str, ExportScope, Vec<Uuid>, MembershipError)> = vec![
        (
            "member_out",
            ExportScope::All,
            vec![],
            MembershipError::EmptySelection,
        ),
        (
            "none_in",
            ExportScope::All,
            vec![m("admin_in")],
            MembershipError::NotAuthorized,
        ),
        (
            "guest_in",
            ExportScope::Fau,
            vec![m("admin_in")],
            MembershipError::NotAuthorized,
        ),
        (
            "member_out",
            ExportScope::Group(w.closed),
            vec![m("member_in")],
            MembershipError::UnknownGroup,
        ),
        (
            "member_out",
            ExportScope::Group(unknown),
            vec![m("member_in")],
            MembershipError::UnknownGroup,
        ),
        (
            "member_out",
            ExportScope::Group(w.other),
            vec![unknown],
            MembershipError::UnknownGroup,
        ),
        // guest_out is in `other`, which member_out cannot read.
        (
            "member_out",
            ExportScope::All,
            vec![m("guest_out")],
            MembershipError::UnknownMembership,
        ),
        (
            "member_out",
            ExportScope::Group(w.open),
            vec![m("guest_in"), m("member_out")],
            MembershipError::UnknownMembership,
        ),
        // A guest never reaches the FAU-wide section's people.
        (
            "guest_in",
            ExportScope::All,
            vec![m("member_out")],
            MembershipError::UnknownMembership,
        ),
        (
            "guest_in",
            ExportScope::All,
            vec![unknown],
            MembershipError::UnknownMembership,
        ),
        (
            "admin_out",
            ExportScope::All,
            vec![other_fau.admin_membership_id],
            MembershipError::UnknownMembership,
        ),
        // A person with no standing today is not listed, even though still in the group.
        (
            "admin_out",
            ExportScope::Group(w.open),
            vec![m("none_in")],
            MembershipError::UnknownMembership,
        ),
    ];
    for (viewer, scope, ids, want) in rows {
        let got = export_addresses(
            &pool,
            w.viewer(viewer),
            export(ExportPurpose::Copy, scope, &ids),
            at(T0),
        )
        .await;
        assert_eq!(got, Err(want.clone()), "{viewer} {scope:?}");
    }
    assert_eq!(exported(&pool).await, 0);

    // The same person from another FAU's viewer: refused as having no standing here.
    let crossed = Viewer {
        tenant_id: w.fau.tenant_id,
        membership_id: other_fau.admin_membership_id,
    };
    assert_eq!(
        export_addresses(
            &pool,
            crossed,
            export(ExportPurpose::Copy, ExportScope::All, &[m("admin_in")]),
            at(T0),
        )
        .await,
        Err(MembershipError::NotAuthorized)
    );
}

#[tokio::test]
async fn a_frozen_fau_still_hands_over_addresses() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    sqlx::query("update tenants set frozen_at = now() where id = $1")
        .bind(w.fau.tenant_id)
        .execute(&pool)
        .await
        .unwrap();
    let got = export_addresses(
        &pool,
        w.viewer("member_in"),
        export(
            ExportPurpose::Mailto,
            ExportScope::Fau,
            &[w.membership("admin_out")],
        ),
        at(T0),
    )
    .await
    .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(exported(&pool).await, 1);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p fau-app --test address_export`
Expected: compile errors, `cannot find function export_addresses` and `no variant EmptySelection`.

- [ ] **Step 3: Write the implementation**

Apply to `backend/crates/persistence/src/membership/error.rs`:

```diff
--- a/backend/crates/persistence/src/membership/error.rs
+++ b/backend/crates/persistence/src/membership/error.rs
@@ -143,6 +143,8 @@ pub enum MembershipError {
     /// running or yet to start -- so it holds no name (Erik's D3, 28 September 2026).
     #[error("the membership has ended")]
     MembershipEnded,
+    #[error("nothing was selected")]
+    EmptySelection,
 
     // Infrastructure.
     #[error("the operating system's random source failed")]
```

Create `backend/crates/persistence/src/membership/export.rs`:

```rust
//! Handing over a selection's addresses for "Skriv e-post" or "Kopier adresser" (groups
//! design §4.3, §4.4; #3502), audited in the same transaction.

use std::collections::HashSet;

use fau_domain::directory::listing::SectionId;
use fau_domain::time::Moment;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::authz::Viewer;
use super::directory::{load, DirectoryAddress, DirectoryRead};
use super::error::MembershipError;
use super::sql::{write_audit, Audit};

/// "Skriv e-post" or "Kopier adresser" (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPurpose {
    Mailto,
    Copy,
}

impl ExportPurpose {
    fn code(self) -> &'static str {
        match self {
            ExportPurpose::Mailto => "mailto",
            ExportPurpose::Copy => "copy",
        }
    }
}

/// Which part of the directory the selection was made in: the screen's group filter, or
/// none of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportScope {
    /// Every section the viewer sees.
    All,
    Fau,
    Group(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressExport {
    pub purpose: ExportPurpose,
    pub scope: ExportScope,
    /// The people ticked, in order; a person ticked in two groups may appear twice.
    pub membership_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedRecipient {
    pub membership_id: Uuid,
    pub address: DirectoryAddress,
}

/// Hands over the addresses of a selection for a `mailto:` link or a copy, and audits it
/// in the same transaction (§4.4): the purpose, the count and the scope, **never the
/// addresses** and never who was selected, because pulling out a batch of addresses is
/// itself a disclosure. Each person comes back once, in first-selected order; the session
/// decrypts contact addresses and builds the link with `fau_domain::directory::address`.
///
/// Refusals, in this order, so none reveals more than the one before it:
/// 1. `EmptySelection`;
/// 2. `NotAuthorized`: no standing, or a guest asking for the FAU-wide section;
/// 3. `UnknownGroup`: a group the viewer cannot read, exactly as for an id that does not
///    exist;
/// 4. `UnknownMembership`: a selected person the scope does not list for this viewer,
///    whether or not the id exists elsewhere.
///
/// Allowed while the FAU is frozen: reading continues (ADR-003 7a).
pub async fn export_addresses(
    pool: &PgPool,
    viewer: Viewer,
    req: AddressExport,
    at: Moment,
) -> Result<Vec<ExportedRecipient>, MembershipError> {
    if req.membership_ids.is_empty() {
        return Err(MembershipError::EmptySelection);
    }
    let mut tx = pool.begin().await?;
    // One snapshot for the authorization and the read, as in `read_transaction`, but
    // writable: the audit entry commits with it.
    sqlx::query("set transaction isolation level repeatable read")
        .execute(&mut *tx)
        .await?;
    let read = load(&mut tx, viewer, at).await?;
    let in_scope = match req.scope {
        ExportScope::Fau => {
            scope_members(&read, req.scope).ok_or(MembershipError::NotAuthorized)?
        }
        ExportScope::Group(_) => {
            scope_members(&read, req.scope).ok_or(MembershipError::UnknownGroup)?
        }
        ExportScope::All => scope_members(&read, req.scope).unwrap_or_default(),
    };
    let mut seen = HashSet::new();
    let mut chosen = Vec::new();
    for id in &req.membership_ids {
        if !in_scope.contains(id) {
            return Err(MembershipError::UnknownMembership);
        }
        if seen.insert(*id) {
            chosen.push(*id);
        }
    }
    let recipients: Vec<ExportedRecipient> = chosen
        .iter()
        .map(|id| {
            let p = read
                .people
                .iter()
                .find(|p| p.membership_id == *id)
                .expect("every section member is a listed person");
            ExportedRecipient {
                membership_id: *id,
                address: p.address.clone(),
            }
        })
        .collect();
    let (scope, group_id) = match req.scope {
        ExportScope::All => ("all", None),
        ExportScope::Fau => ("fau", None),
        ExportScope::Group(g) => ("group", Some(g)),
    };
    write_audit(
        &mut tx,
        at,
        Audit::member(
            viewer.tenant_id,
            viewer.membership_id,
            "directory.addresses_exported",
            "tenant",
            viewer.tenant_id,
            json!({
                "purpose": req.purpose.code(),
                "recipient_count": recipients.len(),
                "scope": scope,
                "group_id": group_id,
            }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(recipients)
}

/// The members of the section `scope` names, if the viewer may see it.
fn scope_members(read: &DirectoryRead, scope: ExportScope) -> Option<HashSet<Uuid>> {
    let pick = |id: SectionId| {
        read.sections
            .iter()
            .find(|s| s.section == id)
            .map(|s| s.members.iter().copied().collect())
    };
    match scope {
        ExportScope::All => Some(read.people.iter().map(|p| p.membership_id).collect()),
        ExportScope::Fau => pick(SectionId::Fau),
        ExportScope::Group(g) => pick(SectionId::Group(g)),
    }
}
```

Apply to `backend/crates/persistence/src/membership/mod.rs`:

```diff
--- a/backend/crates/persistence/src/membership/mod.rs
+++ b/backend/crates/persistence/src/membership/mod.rs
@@ -29,6 +29,7 @@
 mod directory;
 mod error;
 mod events;
+mod export;
 mod groups;
 mod handover;
 mod invitations;
@@ -48,6 +49,7 @@
 };
 pub use error::{ExistingFau, MembershipError};
 pub use events::{Change, Hub, HubClock, Subscription, EVENTS_CHANNEL, SUBSCRIPTION_BUFFER};
+pub use export::{export_addresses, AddressExport, ExportPurpose, ExportScope, ExportedRecipient};
 pub use groups::{
     add_group_member, archive_group, create_group, get_group, list_group_members, list_groups,
     remove_group_member, rename_group, set_group_visibility, ArchiveGroup, CreateGroup,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p fau-app --test address_export`
Expected: PASS, 3 tests.

Mutation check, run while planning: skip the scope check (`if false && !in_scope.contains(id)`), and `a_selection_outside_what_the_viewer_sees_is_refused_and_writes_nothing` fails. Moving the membership loop above the scope match makes the unknown-group rows answer `UnknownMembership`, and the same test fails.

- [ ] **Step 5: Format and lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from `fmt`, and clippy finishes with no warnings.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/persistence/src/membership/export.rs \
        backend/crates/persistence/src/membership/error.rs \
        backend/crates/persistence/src/membership/mod.rs \
        backend/crates/app/tests/address_export.rs
git commit -m "Audit every export of directory addresses with a count, never the addresses (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: The directory end to end under the record key; record the rulings; full verification

**Files:**
- Modify: `backend/crates/app/tests/key_chain.rs`
- Modify: `docs/planning-decisions.md`

**Interfaces:**
- Consumes everything above:
  - `fau_keys::data_key` with `Unit::Record`, and `fau_crypto::{encrypt, decrypt, Aad}`;
  - `prepare_acceptance`, `accept_invitation` and `set_display_name`;
  - `member_directory` and `export_addresses`;
  - `arrange`, `NameCollator` and `Recipients`.
- Produces: the proof that a name and an address are record-key envelopes bound to their membership, and that the session's path from the directory to a link works. This is what the screen card copies.

- [ ] **Step 1: Write the test**

Apply to `backend/crates/app/tests/key_chain.rs`:

```diff
--- a/backend/crates/app/tests/key_chain.rs
+++ b/backend/crates/app/tests/key_chain.rs
@@ -7,7 +7,12 @@
 use common::membership::*;
 use common::TestDb;
 use fau_crypto::{decrypt, encrypt, Aad, Unit};
-use fau_domain::membership::vocabulary::{GroupName, Visibility};
+use fau_crypto::{Ciphertext, DataKey};
+use fau_domain::directory::address::{CopySeparator, Mailto, RecipientField, Recipients};
+use fau_domain::directory::collation::NameCollator;
+use fau_domain::directory::listing::{arrange, Person, Section, ShownName};
+use fau_domain::email::Email;
+use fau_domain::membership::vocabulary::{CapabilityClass, DisplayName, GroupName, Visibility};
 use fau_keys::{data_key, Auth, DataKeyError, KeyCache, KeyError, Keys, KeysConfig};
 use fau_persistence::membership::*;
 use jiff::SignedDuration;
@@ -217,3 +222,247 @@
     let elsewhere = Aad::new(fau.tenant_id, table, column, Uuid::now_v7());
     assert!(decrypt(&key, &elsewhere, &listed[0].encrypted_name).is_err());
 }
+
+fn seal(
+    key: &DataKey,
+    tenant: Uuid,
+    aad: (&'static str, &'static str),
+    row: Uuid,
+    text: &str,
+) -> Ciphertext {
+    encrypt(key, &Aad::new(tenant, aad.0, aad.1, row), text).unwrap()
+}
+
+fn open(
+    key: &DataKey,
+    tenant: Uuid,
+    aad: (&'static str, &'static str),
+    row: Uuid,
+    ct: &Ciphertext,
+) -> String {
+    decrypt(key, &Aad::new(tenant, aad.0, aad.1, row), ct)
+        .unwrap()
+        .as_str()
+        .to_owned()
+}
+
+/// Groups design §4 end to end (#3502): names and contact addresses enter under the record
+/// key bound to their membership, nothing readable reaches Postgres, and the session turns
+/// the directory into an ordered listing and a `mailto:` link.
+#[tokio::test]
+async fn the_directory_end_to_end_under_the_record_key() {
+    let db = TestDb::migrated().await;
+    let pool = db.app_pool().await;
+    let t0 = at(T0);
+    let fau = active_fau(&pool, "admin@example.test", t0).await;
+    let tenant = fau.tenant_id;
+    let keys = keys().await;
+    let cache = KeyCache::new(
+        keys.clone(),
+        Arc::new(jiff::Timestamp::now),
+        SignedDuration::from_mins(30),
+    );
+    let key = {
+        let mut c = pool.acquire().await.unwrap();
+        data_key(
+            &mut c,
+            &keys,
+            &cache,
+            Uuid::now_v7(),
+            &Unit::Record { tenant },
+        )
+        .await
+        .unwrap()
+    };
+
+    // The admin names themself; two people join with names, one with a contact address.
+    set_display_name(
+        &pool,
+        SetDisplayName {
+            tenant_id: tenant,
+            actor_membership_id: fau.admin_membership_id,
+            membership_id: fau.admin_membership_id,
+            encrypted_display_name: seal(
+                &key,
+                tenant,
+                DISPLAY_NAME_AAD,
+                fau.admin_membership_id,
+                "Åse Admin",
+            ),
+        },
+        t0,
+    )
+    .await
+    .unwrap();
+    let mut joined = Vec::new();
+    for (address, name, contact) in [
+        (
+            "kari@example.test",
+            "Kari Nordmann",
+            Some("kari.privat@example.no"),
+        ),
+        ("oystein@example.test", "Øystein", None),
+    ] {
+        let token = issue_invitation(
+            &pool,
+            IssueInvitation {
+                tenant_id: tenant,
+                actor_membership_id: fau.admin_membership_id,
+                recipient: email(address),
+                roles: vec![OfferedRole {
+                    role: new_role("Medlem", CapabilityClass::Member),
+                    period: period(day(2026, 9, 1), day(2027, 9, 1)),
+                }],
+                handover_grant_id: None,
+                message: None,
+            },
+            t0,
+        )
+        .await
+        .unwrap()
+        .token
+        .expose()
+        .to_owned();
+        let target = prepare_acceptance(&pool, &token, &verified(address))
+            .await
+            .unwrap();
+        let m = target.membership_id;
+        let name = DisplayName::parse(name).unwrap();
+        accept_invitation(
+            &pool,
+            AcceptInvitation {
+                token,
+                acceptor: verified(address),
+                admin_end_override: None,
+                profile: MemberProfile {
+                    membership_id: m,
+                    encrypted_display_name: seal(&key, tenant, DISPLAY_NAME_AAD, m, name.as_str()),
+                    encrypted_contact_email: contact.map(|c| {
+                        seal(
+                            &key,
+                            tenant,
+                            CONTACT_EMAIL_AAD,
+                            m,
+                            Email::parse(c).unwrap().as_str(),
+                        )
+                    }),
+                },
+            },
+            t0,
+        )
+        .await
+        .unwrap();
+        joined.push(m);
+    }
+    let (kari, oystein) = (joined[0], joined[1]);
+
+    // Nothing readable in Postgres.
+    let stored: Vec<Vec<u8>> = sqlx::query_scalar(
+        "select coalesce(encrypted_display_name, '') || coalesce(encrypted_contact_email, '')
+           from memberships where tenant_id = $1",
+    )
+    .bind(tenant)
+    .fetch_all(&pool)
+    .await
+    .unwrap();
+    for bytes in &stored {
+        for plain in ["Kari", "Admin", "privat", "ystein"] {
+            assert!(
+                !bytes.windows(plain.len()).any(|w| w == plain.as_bytes()),
+                "{plain}"
+            );
+        }
+    }
+
+    // The session decrypts the directory and orders it for Bokmål: Æ, Ø, Å after Z.
+    let viewer = Viewer {
+        tenant_id: tenant,
+        membership_id: kari,
+    };
+    let read = member_directory(&pool, viewer, t0).await.unwrap();
+    let people: Vec<Person> = read
+        .people
+        .iter()
+        .map(|p| Person {
+            membership_id: p.membership_id,
+            name: match &p.name {
+                MemberName::Named(ct) => ShownName::Named(
+                    DisplayName::parse(&open(&key, tenant, DISPLAY_NAME_AAD, p.membership_id, ct))
+                        .unwrap(),
+                ),
+                _ => ShownName::Unnamed,
+            },
+            address: match &p.address {
+                DirectoryAddress::Contact(ct) => {
+                    Email::parse(&open(&key, tenant, CONTACT_EMAIL_AAD, p.membership_id, ct))
+                        .unwrap()
+                }
+                DirectoryAddress::Login(e) => e.clone(),
+            },
+            is_guest: p.is_guest,
+            roles: p.roles.clone(),
+            groups: p.group_ids.clone(),
+        })
+        .collect();
+    let sections: Vec<Section> = read
+        .sections
+        .iter()
+        .map(|s| Section {
+            id: s.section,
+            title: None,
+            members: s.members.clone(),
+        })
+        .collect();
+    let listing = arrange(sections, people, &NameCollator::for_locale("nb-NO"));
+    assert_eq!(
+        listing.sections[0].members,
+        [kari, oystein, fau.admin_membership_id]
+    );
+
+    // "Skriv e-post" for everyone: the audited export, decrypted, becomes one link.
+    let exported = export_addresses(
+        &pool,
+        viewer,
+        AddressExport {
+            purpose: ExportPurpose::Mailto,
+            scope: ExportScope::Fau,
+            membership_ids: listing.sections[0].members.clone(),
+        },
+        t0,
+    )
+    .await
+    .unwrap();
+    let recipients = Recipients::new(exported.iter().map(|r| match &r.address {
+        DirectoryAddress::Contact(ct) => {
+            Email::parse(&open(&key, tenant, CONTACT_EMAIL_AAD, r.membership_id, ct)).unwrap()
+        }
+        DirectoryAddress::Login(e) => e.clone(),
+    }));
+    assert_eq!(recipients.default_field(), RecipientField::To);
+    assert_eq!(
+        recipients.mailto(RecipientField::To),
+        Mailto::Link(
+            "mailto:kari.privat@example.no,oystein@example.test,admin@example.test".into()
+        )
+    );
+    assert_eq!(
+        recipients.copy_text(CopySeparator::Comma),
+        "kari.privat@example.no, oystein@example.test, admin@example.test"
+    );
+
+    // Bound to its row: Kari's name does not open as Øystein's.
+    let kari_name = match &read
+        .people
+        .iter()
+        .find(|p| p.membership_id == kari)
+        .unwrap()
+        .name
+    {
+        MemberName::Named(ct) => ct.clone(),
+        other => panic!("{other:?}"),
+    };
+    let (t, c) = DISPLAY_NAME_AAD;
+    assert!(decrypt(&key, &Aad::new(tenant, t, c, oystein), &kari_name).is_err());
+    let (t, c) = CONTACT_EMAIL_AAD;
+    assert!(decrypt(&key, &Aad::new(tenant, t, c, kari), &kari_name).is_err());
+}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p fau-app --test key_chain`
Expected: PASS, 3 tests (the new `the_directory_end_to_end_under_the_record_key` included). It needs the compose `openbao` with the dev policies. If it fails with `NotFound` or `permission denied`, check `docker compose -f /workspace/compose.yaml ps openbao` before touching code.

Not a TDD step, but a real check: move the ciphertext to another membership id and decryption fails, which is the test's last three assertions.

- [ ] **Step 3: Record the rulings in `docs/planning-decisions.md`**

Append this section at the end of the file:

```markdown
## #3502 built: the member directory, rulings for Erik's review — 28 September 2026

Built from the accepted #3500 design (§4) on branch `directory-3502`. The plan's rulings are in
docs/superpowers/plans/2026-09-28-member-directory-3502.md, and these are the ones that change
behaviour or bind later work. All are open to challenge:

- **No screen yet.** #3502 builds the schema, the domain rules and the persistence functions.
  The htmx screen, its routes and the TypeScript selection module wait for #3417's sessions.
- **Names and contact addresses are record-key envelopes on `memberships`** (migration 0008),
  bound to the membership id. Acceptance is therefore two calls: `prepare_acceptance` names the
  FAU and the membership, the session encrypts, and `accept_invitation` stores. A mismatch is
  refused, never stored. The registrant's name is captured at activation the same way.
- **A name is valid only while the membership is active (Erik's D3, 28 September).** When the
  membership ends (it is revoked, or no role is running or still to come), the name is cleared
  like the contact address. Revocation clears both in the same statement, and two database
  checks enforce it. A daily sweep clears a membership whose roles ran out, at once and without
  the account's three-month grace. There is therefore no name history.
- **History shows an ended membership as its role and year** ("Leder 2025–2026"), computed from
  the role assignments when history is rendered, never stored. The role is the one held on the
  day of the event: admin class first, then the earliest start. After every role has ended, it
  is the last role that ended. It is one role, without unit or cohort. The years are the
  calendar years the assignment was actually held. The catalogue strings are proposals:
  "{role} {year}" and "{role} {firstYear}–{lastYear}".
- **Who edits.** A member edits their own name, and an admin corrects the name of anyone still
  active. An ended membership takes no name (`MembershipEnded`). Only the member sets their
  contact address.
- **Article 17 erasure** is a storage step for #3426: both fields go, the person is not listed,
  history shows "Tidligere medlem" (not even role and year, since a single-holder role with its
  year identifies the person), and the old membership cannot be rejoined.
- **What the directory shows.** The FAU-wide section, for members and admins only, never lists
  a guest. Beyond it, each group the viewer may read, through the same rule and SQL as the group
  reads. Only people with standing today are listed. A group the viewer cannot read never
  appears as part of a person.
- **Collation is ICU4X** (`icu_collator` 2.3), already mostly in the dependency tree. Bokmål and
  Nynorsk are tailored. Sámi falls back to the root order until ICU4X data is generated for it,
  which D4 places at the time a Sámi locale is added.
- **Links and copying.** The `mailto:` link joins addresses with a bare `,` (RFC 6068), and the
  copy text with `, `, with `; ` ready for Outlook. Bcc is the default above 10 distinct
  addresses. The length guard allows 1,800 characters and refuses 1,801.
- **Every export is server-side and audited:** the purpose, the count of people and the scope
  (all, the FAU-wide section, or one group). No addresses and no member ids. Allowed while
  frozen.

**Open questions for Erik:** none from this card. D3 closed the name-history question, and D4
closed the Sámi one.

**Spec against code:** ADR-003 §6 still lists member names as plaintext. The accepted #3500
spec and the key-service design encrypt them, and the later documents win. Spec §4.2's "the
name stays after the membership ends" is overridden by D3 and needs updating in the spec. Spec
§4.3's "`, ` per RFC 6068" is split: `,` in the link and `, ` in the copy text.
```

- [ ] **Step 4: Full verification**

First `ps aux | grep '[c]argo test'` (must be empty). Then, from `/workspace/backend` with the three test variables exported:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --features fau-app/test-routes -- -D warnings
cargo test --workspace
cargo test -p fau-app --features test-routes --test panic_handling --test shutdown
```

Expected: formatting and both clippy runs are clean, and every test passes. That includes these new test binaries:
- `directory_schema` (4);
- `member_profile` (7);
- `profile_edits` (6);
- `directory_retention` (3);
- `directory` (4);
- `address_export` (3).

It also includes the new `authorization` and `key_chain` tests and the 28 domain `directory` unit tests (6 of them in `history`). The pre-existing suites must be unchanged in outcome, apart from the two raw-SQL revocation fixtures that Task 4 re-fixtures. The amended dry run passed 760 tests in `cargo test --workspace`.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/app/tests/key_chain.rs docs/planning-decisions.md
git commit -m "Prove the directory end to end under the record key; record the #3502 rulings (#3502)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

After this, the controller:
- runs the final whole-branch review (superpowers:requesting-code-review);
- updates #3502 in Favro, attaching this plan and stating the rulings, including the Rulings (D3), in the comment itself, since Erik reads Favro, not the repo;
- leaves merging and pushing to Erik.

---

## Self-review (done while writing)

**Dry run.** Before handing over, the plan's code was applied to a throwaway copy of `backend/` at `2c0fe6b` outside the repository. The results:
- The copy was rebuilt **task by task** from these exact blocks (states T1–T9). For each state, `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` were clean, and that task's test binaries passed.
- On the final state, `cargo test --workspace --no-fail-fast` passed 748 tests against the compose `db` and `openbao`. The `test-routes` clippy run and suites were clean.
- `image.rs` needs the repository's `Dockerfile` beside `backend/`. In the throwaway copy it was copied in; in the repository it is already there.
- Five mutations were applied and each was caught (Tasks 7 and 8, and the sweep's tenant pairing in Task 6); then the code was restored.

**D3 amendment dry run (28 September 2026).** Tasks 3b–9 were re-verified on a throwaway copy of `backend/` (and the `Dockerfile`) taken from branch HEAD `b72bb55`, so Tasks 1–3 as built, with Task 3's fix round `290a814`, were the base.
- Each task was applied from this document's own blocks by a script. It writes every `Create`/`Overwrite` block, applies every `Apply to` diff with `git apply`, and runs Task 4's perl command. The result was compared with the tested tree: identical for Tasks 3b, 4, 5 and 6.
- Task 3b's RED was seen (1 of 4 failing, on `unwrap_err` of an accepted revocation) before the migration was amended.
- Per task, its binaries passed: `directory_schema` 4, `migrations` 25, `schema_review` 6, `membership_schema` 13; Task 4's seven suites plus `roles` and the persistence unit tests; `profile_edits` 6; `directory_retention` 3 and the domain's 28 `directory` tests; `directory` 4, `authorization` 6, `group_reads` 4, `groups` 9; `address_export` 3.
- After Task 4 and on the final state, `cargo test --workspace --no-fail-fast` passed 736 and 760 tests. On the final state, `fmt --check`, both clippy runs and the `test-routes` suites were clean.
- Mutations caught:
  - Task 3b: the name check dropped;
  - Task 5: an ended target let through;
  - Task 6: the running-role date test dropped, `ra.revoked_at is null` dropped, and `member_names` ignoring "ended";
  - Task 6's domain: the admin class ranked last, and the earliest end picked instead of the latest.
- **Correction to the earlier dry run:** dropping `ra.tenant_id = m.tenant_id` from the sweep's predicate is *not* caught, and cannot be, because membership ids are unique across FAU-er. Task 6 now says so, rather than claiming it.
- Found by the dry run, not by reading: two existing tests (`invitations.rs`, `roles.rs`) revoke a membership with raw SQL. Under Task 3b's check they must clear the name too. Task 4 now carries both fixture diffs.
- The execution ledger's rulings Q1–Q8 keep their task numbers, because the new task is 3b. They are carried in dispatches as before. None of them conflicts with D3; Q4's extra `member_names` viewer rows still apply to the rewritten Task 6 test.

**Spec coverage (§4, §8, §10):**

| Requirement | Where it lands |
|---|---|
| §4.1 `display_name` per membership, required at acceptance, member edits and admin corrects | Tasks 1, 4, 5 (R3–R6) |
| §4.1 `contact_email` optional, login email shown when empty, never mailed, not verified | Tasks 1, 4, 5, 7 (`DirectoryAddress::Login`) |
| §4.1 both encrypted under the FAU key | Global constraints; Task 9 end to end |
| §4.1 what a person represents: current roles plus unit or cohort, plus groups; no past roles | Task 7 (`roles`, `group_ids`; the past-role assertion) |
| §4.2 contact address disappears when the membership ends | Task 4 (revocation, database check), Task 6 (sweep) |
| ~~§4.2 name survives the end of the membership~~ D3: the name disappears when the membership ends; history shows role and year | Task 3b (database check), Task 4 (revocation), Task 5 (no edits), Task 6 (sweep, `member_names` → `Ended`, `role_label`) |
| §4.2 Article 17 replaces the name with "Tidligere medlem" | Task 6 (R9, D3-4: not even role and year) |
| §4.2 crypto-shredding removes everything | Record key (key-service §3.1); nothing new |
| §4.3 grouped by group, guests marked, locale-aware collation | Tasks 3, 7 (R11–R13) |
| §4.3 a person selected through two groups appears once | Tasks 2, 3, 8 |
| §4.3 To/Bcc, Bcc above 10 | Task 2 (R16) |
| §4.3 copy via clipboard with a fallback | Task 2 (`copy_text`); the clipboard is the screen card's |
| §4.3 length guard at about 1,800 | Task 2 (R15) |
| §4.3 `, ` per RFC 6068, `; ` if Outlook needs it | Task 2 (R14) |
| §4.4 audit entry with count and group, never addresses | Task 8 (R17) |
| §4.4 guests see only their own groups; members see guests in groups with their address | Task 7 (matrix), Task 8 (export rows) |
| §8 #3418 row: display name captured at acceptance | Task 4 |
| §8 #3439 row: Bokmål strings, collation per locale | R20, Task 3 |
| §8 privacy notice and DPA | "What later cards get" |
| §10 directory: guard at the boundary; once through two groups; ~~name survives while address goes~~ both go, role and year remain (D3); erasure; audit count | Tasks 2, 3/8, 3b/4/6, 6, 8 |
| §10 matrix: directory entry rows | Task 7 (`directory_entries_in_the_matrix`) |

The screen itself (htmx, "Velg alle", the filter and the clipboard call) is deliberately not built (R1).

**Placeholder scan.** No TBD, TODO or "similar to Task N". Every code step carries its code or an exact diff taken from the verified states. The one mechanical step (Task 4, Step 7) is an exact command with its expected counts.

**Type consistency, checked across tasks:**
- `MemberProfile { membership_id, encrypted_display_name, encrypted_contact_email }`;
- `AcceptanceTarget { tenant_id, membership_id }`;
- `SetDisplayName { tenant_id, actor_membership_id, membership_id, encrypted_display_name }`;
- `SetContactEmail { tenant_id, membership_id, encrypted_contact_email }`;
- `MemberName::{Named, Unnamed, Ended(Vec<HeldRole>), Former}` and `DirectoryAddress::{Contact, Login}`;
- `HeldRole { name, class, from, until }`, `RoleLabel { role, first_year, last_year }` and `role_label(&[HeldRole], Date) -> Option<RoleLabel>`;
- `membership_ended(&str) -> String` (Task 5), used by `set_display_name`, `clear_ended_profiles` and `member_names`;
- `MembershipError::MembershipEnded` (Task 4, first returned in Task 5);
- `DirectorySection { section: SectionId, encrypted_group_name, members }`;
- `AddressExport { purpose, scope, membership_ids }` and `ExportScope::{All, Fau, Group}`;
- `Recipients::{new, mailto, copy_text, default_field}`;
- `arrange(Vec<Section>, Vec<Person>, &NameCollator) -> Listing`.
