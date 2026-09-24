-- 0003: the membership foundation. Implements the data model in section 9 of
-- docs/fau-creation-and-membership-flow.md (#3413), on the rules 0002 established:
--   * every reference between tenant data uses the composite key (tenant_id, id);
--   * explicit grants to the runtime role, nothing by default;
--   * no key material of any kind -- invitation tokens are stored as SHA-256 hashes;
--   * half-open date ranges [starts_on, ends_on_exclusive) with a mandatory end.
--
-- The schools register and tenants.school_id's foreign key are #3441's and are
-- deliberately absent. docs/school-register-design.md makes schools a global table keyed
-- on our own UUIDs, so the FK will be a plain `references schools (id)`, not the
-- tenant-scoped composite form 0002's comment anticipated. The register migration adds
-- only that FK: the one-live-FAU-per-school index below already exists and must not be
-- created a second time under the register design's name.

-- 1. Tenants: one live FAU per school, and a freeze marker.

-- A pending FAU counts (spec 3.2); a closed one does not block a new one.
create unique index tenants_one_live_per_school on tenants (school_id)
  where status in ('pending', 'active') and school_id is not null;

-- ADR-003 decision 7a freezes an FAU on a delete request. The freeze flow itself is not
-- built here; the column exists so invitation issuance and acceptance can refuse a
-- frozen FAU, as spec 5.1 requires.
alter table tenants add column frozen_at timestamptz;

-- 2. Pending-signup data (spec 3.1, 3.3). A separate table rather than columns on
-- tenants: these values exist only while the FAU is pending, two of them are personal
-- data, and activation deletes the row, so nothing about the registrant or the leader
-- lingers on the tenant once the account, membership and invitation that replace them
-- exist. Expiry deletes the tenant and the row goes with it.
create table tenant_signups (
  tenant_id               uuid        primary key references tenants (id) on delete cascade,
  registrant_email        text        not null,
  leader_email            text        not null,
  -- The admin end date the registrant chose, as an exclusive end.
  admin_ends_on_exclusive date        not null,
  expires_at              timestamptz not null,
  created_at              timestamptz not null default now()
);
create index tenant_signups_registrant_email_idx on tenant_signups (registrant_email);
create index tenant_signups_expires_at_idx on tenant_signups (expires_at);

-- 3. Audit (spec 9, decision 14; #3412's audit_event). Append-only: the runtime role
-- is granted insert and select, never update or delete. No foreign keys, on purpose:
-- audit outlives the rows it describes (an expired pending FAU is deleted, its audit
-- stays), and an append-only table cannot take part in a cascade.
create table audit_events (
  id                  uuid        primary key,
  -- Null for a global event, such as a signup collision on a school.
  tenant_id           uuid,
  actor_kind          text        not null check (actor_kind in (
                        'member', 'system', 'registrant', 'requester',
                        'recovery_ewb', 'recovery_school_rep')),
  actor_account_id    uuid,
  actor_membership_id uuid,
  -- An action code such as 'invitation.accepted', never a rendered sentence (#3439).
  action              text        not null
    constraint audit_action_is_a_code check (action ~ '^[a-z][a-z_]*(\.[a-z][a-z_]*)+$'),
  subject_type        text        not null
    constraint audit_subject_type_is_a_code check (subject_type ~ '^[a-z][a-z_]*$'),
  subject_id          uuid        not null,
  occurred_at         timestamptz not null,
  -- Bounded parameters: ids, codes, dates, flags. Never personal free text, never a
  -- token (#3412).
  params              jsonb       not null default '{}'::jsonb
    constraint audit_params_are_small
      check (jsonb_typeof(params) = 'object' and octet_length(params::text) <= 2048)
);
create index audit_events_tenant_idx on audit_events (tenant_id, occurred_at);

-- 4. Outbox (spec 2.6, 9). Written in the same transaction as the state change it
-- announces; a sender (#3410) delivers it later. Never carries a token.
-- tenant_id scopes every message about an FAU, so deleting an FAU (ADR-003 decision 7)
-- and an Article 17 erasure can find its queued mail -- recipient addresses included --
-- by column rather than by parsing params. Nullable only for a global message (the
-- signup collision copy to EWB, whose audit entry is global too), and, like
-- audit_events, without a foreign key: a queued message may outlive a deleted pending
-- tenant, and the deletion flow decides what happens to it.
create table outbox (
  id              uuid        primary key,
  tenant_id       uuid,
  template        text        not null
    constraint outbox_template_is_a_code check (template ~ '^[a-z][a-z_]*(\.[a-z][a-z_]*)+$'),
  recipient_email text        not null,
  params          jsonb       not null default '{}'::jsonb
    constraint outbox_params_are_small
      check (jsonb_typeof(params) = 'object' and octet_length(params::text) <= 2048),
  created_at      timestamptz not null,
  sent_at         timestamptz,
  attempts        integer     not null default 0 check (attempts >= 0),
  -- A fixed classification of the last delivery failure, never the provider's message.
  last_error_kind text,
  -- Every template is tenant-scoped except the global ones named here; a new global
  -- template must be added deliberately.
  constraint outbox_tenant_scoped_unless_global
    check (tenant_id is not null or template in ('signup.collision'))
);
create index outbox_unsent_idx on outbox (created_at) where sent_at is null;
create index outbox_tenant_idx on outbox (tenant_id);

-- 5. The recovery-contact seat (spec 6.5, ADR-003 decisions 8 and 9). One row per
-- tenant, written at activation with EWB in the seat.
create table recovery_contacts (
  tenant_id          uuid        primary key references tenants (id),
  holder             text        not null check (holder in ('ewb', 'school_rep')),
  nomination_status  text        not null default 'none'
                       check (nomination_status in ('none', 'nominated', 'confirmed')),
  nominee_email      text,
  nominated_by       uuid,
  nominated_at       timestamptz,
  -- ADR-003 decision 9, step 2: the nominee verified an address on the school's or
  -- municipality's domain.
  domain_verified_at timestamptz,
  -- Step 3: the recorded manual title check -- who checked, when, and how.
  title_checked_by   text,
  title_checked_at   timestamptz,
  title_check_method text        check (title_check_method in ('staff_listing', 'telephone')),
  updated_at         timestamptz not null default now(),
  foreign key (tenant_id, nominated_by) references memberships (tenant_id, id),
  constraint recovery_nominee_matches_status
    check ((nomination_status = 'none') = (nominee_email is null)),
  constraint recovery_confirmation_is_recorded
    check ((nomination_status = 'confirmed') = (domain_verified_at is not null
                                                and title_checked_by is not null
                                                and title_checked_at is not null
                                                and title_check_method is not null)),
  -- EWB keeps the seat until the nominee is confirmed.
  constraint recovery_school_rep_is_confirmed
    check ((holder = 'school_rep') = (nomination_status = 'confirmed'))
);

-- 6. Handover grants (#3412, spec 6.2). The boundary is computed in Rust by
-- fau_domain::membership::rules::handover_boundary and stored, never recomputed in SQL.
create table handover_grants (
  tenant_id            uuid        not null references tenants (id),
  id                   uuid        not null,
  source_assignment_id uuid        not null,
  starts_on            date        not null,
  ends_on_exclusive    date        not null,
  revoked_at           timestamptz,
  created_at           timestamptz not null default now(),
  primary key (tenant_id, id),
  constraint handover_grants_one_per_source unique (tenant_id, source_assignment_id),
  foreign key (tenant_id, source_assignment_id) references role_assignments (tenant_id, id),
  constraint handover_period_is_non_empty check (starts_on < ends_on_exclusive)
);

-- 7. Access requests and replacement proposals: one model (spec 5.4).
create table access_requests (
  tenant_id                  uuid        not null references tenants (id),
  id                         uuid        not null,
  kind                       text        not null check (kind in ('access', 'replacement')),
  -- Who asked: the verified requester, or the proposing member's own address. The
  -- one-open-request limit counts this address.
  requester_email            text        not null,
  -- Who an approval invites: the requester, or the proposed successor.
  invitee_email              text        not null,
  requester_membership_id    uuid,
  replaced_assignment_id     uuid,
  proposed_starts_on         date,
  proposed_ends_on_exclusive date,
  message                    text
    constraint access_request_message_is_short check (char_length(message) <= 500),
  status                     text        not null default 'pending'
    check (status in ('pending', 'approved', 'declined', 'withdrawn', 'lapsed')),
  -- The Europe/Oslo date of creation, for the per-day limit and the 30-day lapse.
  created_on                 date        not null,
  created_at                 timestamptz not null,
  decided_by                 uuid,
  closed_at                  timestamptz,
  primary key (tenant_id, id),
  foreign key (tenant_id, requester_membership_id) references memberships (tenant_id, id),
  foreign key (tenant_id, replaced_assignment_id)  references role_assignments (tenant_id, id),
  foreign key (tenant_id, decided_by)              references memberships (tenant_id, id),
  constraint replacement_names_proposer_and_role
    check ((kind = 'replacement') = (requester_membership_id is not null
                                     and replaced_assignment_id is not null
                                     and proposed_starts_on is not null
                                     and proposed_ends_on_exclusive is not null)),
  constraint access_request_proposed_period_is_non_empty
    check (proposed_starts_on < proposed_ends_on_exclusive),
  constraint access_request_closure_matches_status
    check ((status = 'pending') = (closed_at is null)),
  constraint access_request_decider_matches_status
    check ((status in ('approved', 'declined')) = (decided_by is not null))
);
-- The one-open-request limit, enforced by the database as well as by the code.
create unique index access_requests_one_open_per_address
  on access_requests (tenant_id, requester_email) where status = 'pending';
create index access_requests_created_on_idx on access_requests (tenant_id, created_on);

-- 8. Invitations (spec 5.1).
create table invitations (
  tenant_id              uuid        not null references tenants (id),
  id                     uuid        not null,
  -- SHA-256 of the token. The token itself is returned once and never stored.
  token_hash             bytea       not null,
  mode                   text        not null
    check (mode in ('normal', 'activation', 'handover', 'recovery')),
  recipient_email        text        not null,
  -- The issuing membership. Null for activation (the signup issues it) and recovery
  -- (the recovery contact holds no membership).
  issued_by              uuid,
  handover_grant_id      uuid,
  access_request_id      uuid,
  -- Which seat issued a recovery invitation.
  recovery_holder        text        check (recovery_holder in ('ewb', 'school_rep')),
  expires_at             timestamptz not null,
  accepted_at            timestamptz,
  accepted_membership_id uuid,
  revoked_at             timestamptz,
  created_at             timestamptz not null,
  primary key (tenant_id, id),
  constraint invitations_token_hash_unique unique (token_hash),
  constraint invitation_token_hash_is_sha256 check (octet_length(token_hash) = 32),
  foreign key (tenant_id, issued_by)              references memberships (tenant_id, id),
  foreign key (tenant_id, handover_grant_id)      references handover_grants (tenant_id, id),
  foreign key (tenant_id, access_request_id)      references access_requests (tenant_id, id),
  foreign key (tenant_id, accepted_membership_id) references memberships (tenant_id, id),
  constraint invitation_issuer_matches_mode
    check ((mode in ('normal', 'handover')) = (issued_by is not null)),
  constraint invitation_handover_has_grant
    check ((mode = 'handover') = (handover_grant_id is not null)),
  constraint invitation_recovery_names_seat
    check ((mode = 'recovery') = (recovery_holder is not null)),
  constraint invitation_acceptance_is_complete
    check ((accepted_at is null) = (accepted_membership_id is null)),
  constraint invitation_is_not_accepted_and_revoked
    check (accepted_at is null or revoked_at is null)
);
create index invitations_recipient_idx on invitations (tenant_id, recipient_email);

-- The roles an invitation offers. Each row references a roles row, as #3412's
-- invitation_role does, rather than carrying a copy of a name and class: a role is a
-- position that successive people hold, so the leader invitation, a replacement
-- proposal and a handover invitation all offer the same role the previous holder had,
-- and the capability class can only come from the role itself.
create table invitation_roles (
  tenant_id         uuid not null references tenants (id),
  invitation_id     uuid not null,
  role_id           uuid not null,
  starts_on         date not null,
  ends_on_exclusive date not null,
  primary key (tenant_id, invitation_id, role_id),
  foreign key (tenant_id, invitation_id) references invitations (tenant_id, id),
  foreign key (tenant_id, role_id)       references roles (tenant_id, id),
  constraint invitation_role_period_is_non_empty check (starts_on < ends_on_exclusive)
);

-- 9. Grants. Audit is insert and select only; the outbox is never deleted from by the
-- runtime (purging sent rows is #3410's decision); tenant_signups is deleted on
-- activation, and cascades from a deleted pending tenant.
grant select, insert on audit_events to fau_app;
grant select, insert, update on outbox to fau_app;
grant select, insert, update, delete on tenant_signups to fau_app;
grant select, insert, update on
  recovery_contacts, handover_grants, access_requests, invitations
  to fau_app;
grant select, insert on invitation_roles to fau_app;

insert into schema_contract (version) values (3);
