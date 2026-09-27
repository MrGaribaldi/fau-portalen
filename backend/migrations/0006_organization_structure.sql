-- 0006: the rest of #3412's school structure, which the groups design (#3500, §3.1 and §8)
-- needs before groups can bind to it: school years, cohorts, organization units and the
-- cohorts a unit spans. It also closes the foreign keys 0002 left open on roles.unit_id and
-- roles.cohort_id.
--
-- This is school structure, not a pupil register: no table here names a child. Names are
-- plaintext, like role names (ADR-003 decision 6's plaintext list).
--
-- Out of scope, deliberately:
--   * #3412's unit_relation. Nothing in #3500 uses it.
--   * Any transaction that writes these tables. The initial school configuration flow is
--     its own work, so the runtime role gets select and insert only.
--
-- Unit kinds are English codes. #3412's "gruppe" becomes teaching_group so that it cannot
-- be confused with the FAU groups of migration 0007.

create table school_years (
  tenant_id         uuid        not null references tenants (id),
  id                uuid        not null,
  name              text        not null check (length(name) between 1 and 100),
  -- Local calendar dates, half-open, like every period in this schema.
  starts_on         date        not null,
  ends_on_exclusive date        not null,
  created_at        timestamptz not null default now(),
  primary key (tenant_id, id),
  constraint school_year_period_is_non_empty check (starts_on < ends_on_exclusive)
);

-- A cohort is a stable identity for a year group across school years (#3412), e.g. K2019.
create table cohorts (
  tenant_id  uuid        not null references tenants (id),
  id         uuid        not null,
  name       text        not null check (length(name) between 1 and 100),
  created_at timestamptz not null default now(),
  primary key (tenant_id, id),
  constraint cohorts_name_unique unique (tenant_id, name)
);

-- A unit belongs to one school year, and historical rows are kept: next year's 3A is a new
-- row, so the past keeps showing the structure that applied at the time.
create table organization_units (
  tenant_id      uuid        not null references tenants (id),
  id             uuid        not null,
  school_year_id uuid        not null,
  kind           text        not null check (kind in ('grade', 'class', 'base', 'teaching_group')),
  name           text        not null check (length(name) between 1 and 100),
  created_at     timestamptz not null default now(),
  primary key (tenant_id, id),
  foreign key (tenant_id, school_year_id) references school_years (tenant_id, id),
  constraint organization_units_name_unique unique (tenant_id, school_year_id, name)
);

-- Many-to-many, so a base can span two cohorts at different grades (#3412's example).
create table unit_cohorts (
  tenant_id   uuid     not null references tenants (id),
  unit_id     uuid     not null,
  cohort_id   uuid     not null,
  -- Grunnskole: grades 1 to 10.
  grade_level smallint not null check (grade_level between 1 and 10),
  primary key (tenant_id, unit_id, cohort_id),
  foreign key (tenant_id, unit_id)   references organization_units (tenant_id, id),
  foreign key (tenant_id, cohort_id) references cohorts (tenant_id, id)
);
create index unit_cohorts_cohort_idx on unit_cohorts (tenant_id, cohort_id);

-- The foreign keys 0002 promised. The composite form, never a bare references on the id.
alter table roles
  add constraint roles_unit_fk   foreign key (tenant_id, unit_id)   references organization_units (tenant_id, id),
  add constraint roles_cohort_fk foreign key (tenant_id, cohort_id) references cohorts (tenant_id, id);
create index roles_unit_idx   on roles (tenant_id, unit_id)   where unit_id is not null;
create index roles_cohort_idx on roles (tenant_id, cohort_id) where cohort_id is not null;

grant select, insert on school_years, cohorts, organization_units, unit_cohorts to fau_app;

insert into schema_contract (version) values (6);
