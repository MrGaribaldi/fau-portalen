-- 0007: groups, their members and the guest class (#3501; groups design §3.1).
--
-- Groups are arbitrary: a committee, a year, a working group. They are open to members by
-- default, and an admin can close one. A group may bind to one unit or one cohort (0006).
-- Its membership is then derived from the roles held on that unit or cohort on the current
-- Europe/Oslo date. That derivation is a query (fau_persistence's in_group_sql), never copied
-- rows, so turnover needs no job.
--
-- The name is content ("Oppfølging av sak med rektor"). It is encrypted by the backend
-- under the FAU's record key with AAD (tenant, 'groups', 'encrypted_name', id). This table
-- holds the envelope only, and no key material. The check below is structural: version
-- byte 1, and 41 bytes of nonce and tag around a 1..400-byte name.
create table groups (
  tenant_id      uuid        not null references tenants (id),
  id             uuid        not null,
  encrypted_name bytea       not null,
  visibility     text        not null default 'open' check (visibility in ('open', 'closed')),
  unit_id        uuid,
  cohort_id      uuid,
  created_by     uuid        not null,
  created_at     timestamptz not null,
  -- Archiving is one-way in the MVP: the group stays readable as history, takes no writes.
  archived_at    timestamptz,
  primary key (tenant_id, id),
  foreign key (tenant_id, unit_id)    references organization_units (tenant_id, id),
  foreign key (tenant_id, cohort_id)  references cohorts (tenant_id, id),
  foreign key (tenant_id, created_by) references memberships (tenant_id, id),
  constraint groups_bound_to_at_most_one check (num_nonnulls(unit_id, cohort_id) <= 1),
  constraint groups_name_is_an_envelope
    check (get_byte(encrypted_name, 0) = 1 and octet_length(encrypted_name) between 42 and 512)
);
create index groups_unit_idx   on groups (tenant_id, unit_id)   where unit_id is not null;
create index groups_cohort_idx on groups (tenant_id, cohort_id) where cohort_id is not null;

-- Hand-added members. Removal is soft, because group history is FAU history. Who removed
-- whom is in audit_events.
create table group_members (
  tenant_id     uuid        not null references tenants (id),
  id            uuid        not null,
  group_id      uuid        not null,
  membership_id uuid        not null,
  added_by      uuid        not null,
  added_at      timestamptz not null,
  removed_at    timestamptz,
  primary key (tenant_id, id),
  foreign key (tenant_id, group_id)      references groups (tenant_id, id),
  foreign key (tenant_id, membership_id) references memberships (tenant_id, id),
  foreign key (tenant_id, added_by)      references memberships (tenant_id, id),
  constraint group_member_removed_after_added check (removed_at is null or removed_at >= added_at)
);
create unique index group_members_one_current
  on group_members (tenant_id, group_id, membership_id) where removed_at is null;
create index group_members_membership_idx
  on group_members (tenant_id, membership_id) where removed_at is null;

-- The third capability class. A guest role names exactly one group, and only a guest role
-- names one. A guest in two groups holds two guest roles, or is added to the second by hand.
alter table roles add column group_id uuid;
alter table roles add constraint roles_group_fk
  foreign key (tenant_id, group_id) references groups (tenant_id, id);
-- Same name as 0002's generated one, which the code-set agreement test reads.
alter table roles drop constraint roles_capability_class_check;
alter table roles add constraint roles_capability_class_check
  check (capability_class in ('member', 'admin', 'guest'));
alter table roles add constraint roles_guest_names_a_group
  check ((capability_class = 'guest') = (group_id is not null));
-- A guest reaches only its own groups (groups design §3.3, controller ruling P7): a guest
-- role naming a unit or a cohort would put its holder into every group bound to that unit
-- or cohort through ROLE_FOLLOWS_GROUP, which is broader than "its own groups".
alter table roles add constraint roles_guest_has_no_unit_or_cohort
  check (capability_class <> 'guest' or (unit_id is null and cohort_id is null));
create index roles_group_idx on roles (tenant_id, group_id) where group_id is not null;

-- No delete: groups are archived and members removed softly.
grant select, insert, update on groups, group_members to fau_app;

insert into schema_contract (version) values (7);
