-- 0009: remembering a former member for a chosen period (#3511;
-- docs/member-retention-design.md, Erik's M1-M4 of 29 September 2026). Amends 0008's D3.
--
-- When a membership ends, its display name and contact address are no longer cleared at
-- once. They are kept, hidden, until profile_retained_until, and then the sweep
-- (fau_persistence's clear_ended_profiles) clears both and the date. Others never see a
-- retained name: every read decides "ended" at read time. A person who returns within the
-- period is recognised again; history for others stays role and year.
--
-- * profile_retained_until: the first day the fields are no longer kept (exclusive). Set
--   when the end is noticed, from the account's retention_months; null while active, once
--   cleared, and after an erasure. A check cannot read the clock, so these checks hold the
--   shape and the sweep holds the date.
-- * accounts.retention_months: the member's chosen period, 0 (clear at once, 0008's
--   behaviour), 3 (the default), 6, 12 or 24 months. 0 was refused before.
alter table memberships add column profile_retained_until date;

alter table memberships
  drop constraint memberships_display_name_only_while_current,
  drop constraint memberships_contact_email_only_while_current;
alter table memberships add constraint memberships_display_name_only_while_current_or_retained
  check (revoked_at is null or encrypted_display_name is null
         or profile_retained_until is not null);
alter table memberships add constraint memberships_contact_email_only_while_current_or_retained
  check (revoked_at is null or encrypted_contact_email is null
         or profile_retained_until is not null);
alter table memberships add constraint memberships_retention_needs_a_field
  check (profile_retained_until is null
         or encrypted_display_name is not null or encrypted_contact_email is not null);
alter table memberships add constraint memberships_erasure_clears_retention
  check (name_erased_at is null or profile_retained_until is null);

-- The inline check from 0002 carries PostgreSQL's generated name.
alter table accounts drop constraint accounts_retention_months_check;
alter table accounts add constraint accounts_retention_months_is_allowed
  check (retention_months in (0, 3, 6, 12, 24));

insert into schema_contract (version) values (9);
