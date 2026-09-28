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
