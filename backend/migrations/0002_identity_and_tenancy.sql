-- 0002: the identity and tenancy spine. Reconciles the approved #3412 model with
-- ADR-003 (identity and encryption) and #3439 (localisation).
--
-- Three properties are load-bearing and must not be softened later:
--   1. Every reference between tenant data uses the composite key (tenant_id, id).
--   2. An account may hold several identity mappings at once.
--   3. Role periods are half-open [starts_on, ends_on_exclusive) with a mandatory end.
--
-- No key material of any kind appears in any table. Document and tenant keys live
-- only in the key service (ADR-003 decision 5).

-- Global. Accounts and authentication are not tenant data.
create table accounts (
  id               uuid        primary key,
  email            text        not null unique,
  verified_at      timestamptz,
  disabled_at      timestamptz,
  -- #3439: a BCP 47 tag, nullable because "no preference" is distinct from nb-NO.
  -- Deliberately unconstrained: nothing may hard-code two locales.
  locale           text,
  -- ADR-003 decision 6a. The field ships from the first migration; the screen does not.
  retention_months integer     not null default 3 check (retention_months > 0),
  created_at       timestamptz not null default now()
);

-- ADR-003 decision 3. (issuer, subject) is the key; account_id is NOT unique, which
-- is what allows two providers to run concurrently during a migration.
create table identity_mappings (
  issuer     text        not null,
  subject    text        not null,
  account_id uuid        not null references accounts (id) on delete cascade,
  created_at timestamptz not null default now(),
  primary key (issuer, subject)
);
create index identity_mappings_account_id_idx on identity_mappings (account_id);

create table tenants (
  id             uuid        primary key,
  name           text        not null,
  status         text        not null check (status in ('pending', 'active', 'closed')),
  -- No FK yet -- `school` (#3412) is out of scope for this migration, and it will
  -- itself carry tenant_id. When it lands, this must become a composite foreign key
  -- (tenant_id, school_id) references schools (tenant_id, id), never a bare
  -- references schools (id): every reference between tenant tables uses the
  -- composite key (property 1 above).
  school_id      uuid,
  -- #3439: what this FAU uses for anyone without a personal preference.
  default_locale text        not null default 'nb-NO',
  created_at     timestamptz not null default now()
);

-- From here down every table carries tenant_id and every reference is composite.
create table memberships (
  tenant_id  uuid        not null references tenants (id),
  id         uuid        not null,
  account_id uuid        not null references accounts (id),
  revoked_at timestamptz,
  created_at timestamptz not null default now(),
  primary key (tenant_id, id),
  -- #3412: one membership per account per FAU.
  unique (tenant_id, account_id)
);
-- Looking up every membership an account holds, across tenants -- e.g. to enforce
-- ADR-003 6a's retention rule ("no active membership anywhere") without a full
-- table scan. The unique (tenant_id, account_id) above only serves lookups already
-- scoped to one tenant.
create index memberships_account_id_idx on memberships (account_id);

create table roles (
  tenant_id        uuid not null references tenants (id),
  id               uuid not null,
  name             text not null,
  -- #3412: the privilege follows the explicit class, never the free-text name.
  capability_class text not null check (capability_class in ('member', 'admin')),
  -- No FK yet -- `organization_unit` and `cohort` (#3412) are out of scope for this
  -- migration, and both will carry tenant_id. When they land, these must become
  -- composite foreign keys, (tenant_id, unit_id) references organization_units
  -- (tenant_id, id) and (tenant_id, cohort_id) references cohorts (tenant_id, id),
  -- never a bare references on the id alone.
  unit_id          uuid,
  cohort_id        uuid,
  created_at       timestamptz not null default now(),
  primary key (tenant_id, id)
);

create table role_assignments (
  tenant_id         uuid not null references tenants (id),
  id                uuid not null,
  membership_id     uuid not null,
  role_id           uuid not null,
  -- Local calendar dates in Europe/Oslo. The server's clock decides access.
  starts_on          date not null,
  ends_on_exclusive  date not null,
  revoked_at        timestamptz,
  granted_by        uuid,
  created_at        timestamptz not null default now(),
  primary key (tenant_id, id),
  -- The composite foreign keys are the isolation guarantee that ships now,
  -- ahead of row-level security in #3418.
  foreign key (tenant_id, membership_id) references memberships (tenant_id, id),
  foreign key (tenant_id, role_id)       references roles (tenant_id, id),
  foreign key (tenant_id, granted_by)    references memberships (tenant_id, id),
  constraint role_period_is_non_empty check (starts_on < ends_on_exclusive)
);
create index role_assignments_membership_idx on role_assignments (tenant_id, membership_id);
-- Who holds a given role, and who granted what -- both queried by tenant-scoped
-- role, so tenant_id leads each index the same way the primary key does.
create index role_assignments_role_idx on role_assignments (tenant_id, role_id);
create index role_assignments_granted_by_idx on role_assignments (tenant_id, granted_by);

-- The runtime role gets exactly what it needs on exactly these tables. A future
-- table is unreachable from the application until a migration says otherwise, and
-- finalised history and audit (#3419, #3421) will be granted select and insert only.
grant select, insert, update, delete on
  accounts, identity_mappings, tenants, memberships, roles, role_assignments
  to fau_app;
grant select on schema_contract to fau_app;

insert into schema_contract (version) values (2);
