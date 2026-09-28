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
