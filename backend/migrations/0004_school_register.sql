-- 0004: the school and municipality register (#3441). Implements
-- docs/school-register-design.md sections 4.1-4.6 and 5.4, as decided on
-- 24 September 2026 (docs/planning-decisions.md, "School register decided").
--
-- Global, public reference data: no table here carries tenant_id, and nothing is
-- encrypted -- this is Udir's, Kartverket's, SSB's and Brreg's open data. The one
-- exception, who submitted a school, lives in school_submissions and is personal data
-- whose retention #3426 owns. Brreg FAU addresses are personal data in practice (many
-- are a parent's home address) and are never stored: registered_faus has no address.
--
-- Supersedes 0002's comment on tenants.school_id. That comment predicted a
-- tenant-scoped schools (tenant_id, id) and a composite, deferrable foreign key. The
-- flow spec (section 9) made schools global, so the key below is a plain
-- references schools (id), with no circularity. The one-live-FAU-per-school index
-- already exists (0003, tenants_one_live_per_school) and is not created again.
--
-- Privileges (D9): fau_register alone writes the register. fau_app reads the public
-- tables and may insert only a pending submitted school, its submission and its lookup
-- row -- the insert on schools is confined by row-level security below.

-- 1. Municipalities (4.1).
create table municipalities (
  id                uuid        primary key,
  -- The Norwegian name (Kartverket kommunenavnNorsk), which the slug follows (D3).
  name              text        not null,
  official_name     text,
  county_number     text        not null check (county_number ~ '^[0-9]{2}$'),
  county_name       text        not null,
  slug              text        not null,
  status            text        not null check (status in ('active', 'dissolved')),
  dissolved_on      date,
  -- Svalbard (2100) is 'manual': neither source lists it (D2).
  source            text        not null check (source in ('kartverket', 'manual')),
  source_checked_at timestamptz,
  search_text       text        not null,
  created_at        timestamptz not null default now(),
  updated_at        timestamptz not null default now(),
  constraint municipalities_dissolved_has_date check ((status = 'dissolved') = (dissolved_on is not null)),
  -- <kommunenr>-<navn>, ADR-002 section 3; NULL is unaffected (nothing here is nullable).
  constraint municipalities_slug_format
    check (slug ~ '^[0-9]{4}-[a-z0-9]+(-[a-z0-9]+)*$' and length(slug) <= 85)
);
create unique index municipalities_slug_current on municipalities (slug);

create table municipality_names (
  municipality_id uuid    not null references municipalities (id) on delete restrict,
  name            text    not null,
  language        text    not null,
  priority        integer not null,
  primary key (municipality_id, name)
);

-- A number is reused over time (0716 was Våle, then Re), so it is keyed on
-- (number, valid_from) and a lookup by number must say when.
create table municipality_numbers (
  municipality_id uuid not null references municipalities (id) on delete restrict,
  number          text not null check (number ~ '^[0-9]{4}$'),
  valid_from      date not null,
  valid_until     date,
  primary key (number, valid_from),
  check (valid_until is null or valid_from < valid_until)
);
create unique index municipality_numbers_current on municipality_numbers (number) where valid_until is null;
create index municipality_numbers_municipality_idx on municipality_numbers (municipality_id);

create table municipality_slug_history (
  slug            text        not null,
  municipality_id uuid        not null references municipalities (id) on delete restrict,
  valid_from      timestamptz not null,
  valid_until     timestamptz not null,
  primary key (slug, valid_from),
  check (valid_from < valid_until),
  constraint municipality_slug_history_slug_format
    check (slug ~ '^[0-9]{4}-[a-z0-9]+(-[a-z0-9]+)*$' and length(slug) <= 85)
);

-- 2. Schools (4.2).
create table schools (
  id                   uuid        primary key,
  municipality_id      uuid        not null references municipalities (id) on delete restrict,
  origin               text        not null check (origin in ('register', 'submitted')),
  display_name         text        not null,
  register_name        text,
  display_name_curated boolean     not null default false,
  slug                 text,
  -- listed: from NSR; pending: submitted, awaiting review; verified: submitted and
  -- accepted; rejected; held: an NSR row held back while a match is reviewed (4.5).
  verification         text        not null
    check (verification in ('listed', 'pending', 'verified', 'rejected', 'held')),
  verified_at          timestamptz,
  orgnr                text        unique check (orgnr ~ '^[0-9A-Z]{9}$'),
  ownership            text        check (ownership in ('public', 'private')),
  grade_from           smallint    check (grade_from between 1 and 13),
  grade_to             smallint    check (grade_to between 1 and 13),
  register_language    text,
  website              text,
  street_address       text,
  postcode             text,
  post_town            text,
  in_scope             boolean     not null default true,
  scope_override       boolean,
  status               text        not null check (status in ('active', 'closed')),
  closed_on            date,
  closure_reason       text
    check (closure_reason in ('closed', 'merged', 'duplicate', 'rejected', 'out_of_scope')),
  -- Many-to-one: several closed schools may name one successor (merging FAU-er, #3498).
  successor_id         uuid        references schools (id) on delete restrict,
  source_changed_at    timestamptz,
  last_seen_in_source_at timestamptz,
  search_text          text        not null,
  created_at           timestamptz not null default now(),
  updated_at           timestamptz not null default now(),
  constraint schools_closed_has_date check ((status = 'closed') = (closed_on is not null)),
  constraint schools_reason_only_when_closed check (status = 'closed' or closure_reason is null),
  constraint schools_register_origin_has_orgnr check (origin = 'submitted' or orgnr is not null),
  constraint schools_slug_needs_verification check (slug is null or verification in ('listed', 'verified')),
  constraint schools_grades_ordered check (grade_from is null or grade_to is null or grade_from <= grade_to),
  constraint schools_not_own_successor check (successor_id is null or successor_id <> id),
  -- NULL (unverified) passes: slug ~ '...' on a NULL slug is NULL, which a CHECK accepts.
  constraint schools_slug_format
    check (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' and length(slug) <= 80)
);
-- ADR-002 section 4: school slugs are unique within a municipality, among current slugs.
create unique index schools_slug_current on schools (municipality_id, slug) where slug is not null;
create index schools_municipality_idx on schools (municipality_id);

create table school_orgnr_history (
  orgnr       text        primary key,
  school_id   uuid        not null references schools (id) on delete restrict,
  valid_from  timestamptz not null,
  valid_until timestamptz
);
create index school_orgnr_history_school_idx on school_orgnr_history (school_id);

-- Keyed on the municipality at the time, so an old path still resolves after the
-- school moved municipality in a split.
create table school_slug_history (
  municipality_id uuid        not null references municipalities (id) on delete restrict,
  slug            text        not null,
  school_id       uuid        not null references schools (id) on delete restrict,
  valid_from      timestamptz not null,
  valid_until     timestamptz not null,
  primary key (municipality_id, slug, valid_from),
  check (valid_from < valid_until),
  constraint school_slug_history_slug_format
    check (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' and length(slug) <= 80)
);
create index school_slug_history_school_idx on school_slug_history (school_id);

alter table tenants alter column school_id set not null;
alter table tenants
  add constraint tenants_school_fk foreign key (school_id) references schools (id) on delete restrict;

-- 3. Brreg FAU entities and their links (4.6). No address column, on purpose.
create table registered_faus (
  orgnr                  text        primary key check (orgnr ~ '^[0-9]{9}$'),
  registered_name        text        not null,
  organisation_form      text        not null,
  municipality_number    text        check (municipality_number ~ '^[0-9]{4}$'),
  status                 text        not null check (status in ('active', 'deleted')),
  last_seen_in_source_at timestamptz not null,
  created_at             timestamptz not null default now(),
  updated_at             timestamptz not null default now()
);

create table school_fau_links (
  school_id  uuid        not null references schools (id) on delete restrict,
  fau_orgnr  text        not null references registered_faus (orgnr) on delete restrict,
  method     text        not null check (method in ('address', 'name', 'operator')),
  state      text        not null check (state in ('linked', 'candidate', 'rejected')),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  primary key (school_id, fau_orgnr)
);
create unique index school_fau_links_one_per_school on school_fau_links (school_id) where state = 'linked';
create unique index school_fau_links_one_per_fau on school_fau_links (fau_orgnr) where state = 'linked';
create index school_fau_links_fau_idx on school_fau_links (fau_orgnr);

-- 4. Sync bookkeeping (4.3). NSR payloads only: Brreg payloads carry addresses.
create table register_source_records (
  source            text        not null check (source in ('nsr')),
  external_id       text        not null,
  payload           jsonb       not null,
  payload_sha256    text        not null check (payload_sha256 ~ '^[0-9a-f]{64}$'),
  source_changed_at timestamptz,
  fetched_at        timestamptz not null,
  in_scope          boolean     not null,
  -- Every code domain::register::scope::SCOPE_REASON_CODES lists; keep the two in
  -- sync by hand, since the database cannot read the Rust constant.
  scope_reason      text        not null
    constraint register_source_records_scope_reason_known
    check (scope_reason in (
      'in_scope', 'not_a_school', 'inactive', 'not_grunnskole', 'abroad',
      'adult_education', 'upper_secondary')),
  primary key (source, external_id),
  constraint register_source_records_in_scope_matches_reason
    check (in_scope = (scope_reason = 'in_scope'))
);

create table register_sync_runs (
  id           uuid        primary key,
  kind         text        not null check (kind in ('seed', 'sync', 'dry_run')),
  started_at   timestamptz not null,
  finished_at  timestamptz,
  outcome      text        check (outcome in ('applied', 'no_change', 'aborted', 'failed')),
  counts       jsonb       not null default '{}'::jsonb check (jsonb_typeof(counts) = 'object'),
  abort_reason text,
  constraint register_sync_runs_outcome_when_finished check ((finished_at is null) = (outcome is null))
);

create table register_review_items (
  id              uuid        primary key,
  kind            text        not null check (kind in (
                    'closure_with_fau', 'possible_reregistration', 'possible_submission_match',
                    'municipality_split_or_merge', 'mass_change', 'unknown_municipality_number',
                    'fau_several_at_school', 'fau_several_schools', 'fau_match_conflict',
                    'submission_matches_listed_school')),
  -- set null, not restrict: 4.5's "match" resolution deletes the held NSR row a
  -- possible_submission_match item names, once the submitted school has absorbed
  -- its orgnr and attributes. The held row's own orgnr and name live on in this
  -- item's details, so the reference going null loses nothing the review needs.
  school_id       uuid        references schools (id) on delete set null,
  other_school_id uuid        references schools (id) on delete set null,
  municipality_id uuid        references municipalities (id) on delete restrict,
  fau_orgnr       text        references registered_faus (orgnr) on delete restrict,
  -- Ids, codes, names and orgnrs only; never an address (ruling, 24 September 2026).
  details         jsonb       not null default '{}'::jsonb check (jsonb_typeof(details) = 'object'),
  created_at      timestamptz not null default now(),
  resolved_at     timestamptz,
  resolution      text,
  constraint register_review_items_resolved_together check ((resolved_at is null) = (resolution is null)),
  constraint register_review_items_details_bounded check (octet_length(details::text) <= 2048)
);
create index register_review_items_open_idx on register_review_items (created_at) where resolved_at is null;

-- 5. Submitted schools (4.4) and their immediate lookup (5.4, D9).
create table school_submissions (
  id               uuid        primary key,
  school_id        uuid        not null unique references schools (id) on delete restrict,
  submitted_by     uuid        not null references accounts (id),
  submitted_name   text        not null,
  decision_url     text        not null,
  decision_kind    text        not null
    check (decision_kind in ('municipal_decision', 'udir_private_school_approval')),
  document_title   text,
  retrieval_status text        not null check (retrieval_status in (
                     'fetched', 'not_allowlisted', 'failed', 'quarantined', 'attached_by_reviewer')),
  retrieved_at     timestamptz,
  review_state     text        not null check (review_state in ('pending', 'verified', 'matched', 'rejected')),
  reviewed_at      timestamptz,
  created_at       timestamptz not null default now()
);

create table register_lookups (
  id            uuid        primary key,
  submission_id uuid        not null unique references school_submissions (id) on delete restrict,
  queued_at     timestamptz not null,
  processed_at  timestamptz,
  outcome       text        check (outcome in ('approved', 'review', 'not_found', 'failed')),
  attempts      integer     not null default 0 check (attempts >= 0),
  constraint register_lookups_outcome_when_processed check ((processed_at is null) = (outcome is null))
);
create index register_lookups_queued_idx on register_lookups (queued_at) where processed_at is null;

-- 6. Register mail is global (no tenant): widen 0003's list deliberately.
alter table outbox drop constraint outbox_tenant_scoped_unless_global;
alter table outbox add constraint outbox_tenant_scoped_unless_global
  check (tenant_id is not null or template in (
    'signup.collision', 'register.review_item', 'register.seed_summary',
    'register.submission_approved', 'register.sync_aborted'));

-- 7. Privileges (D9).
grant select, insert, update on
  municipalities, municipality_names, municipality_numbers, municipality_slug_history,
  schools, school_orgnr_history, school_slug_history, registered_faus, school_fau_links,
  register_source_records, register_sync_runs, register_review_items,
  school_submissions, register_lookups
  to fau_register;
-- Only a held row may be deleted (5.3); RLS below narrows it. A review item that
-- named it goes to null on delete, not restrict, so the delete this policy allows
-- always succeeds.
grant delete on schools to fau_register;
-- Kartverket's sync must remove names it has dropped, not merely add new ones.
grant delete on municipality_names to fau_register;
-- Whether a school has an FAU decides between automatic change and review (D8).
grant select on tenants to fau_register;
grant select, insert on audit_events to fau_register;
grant select, insert on outbox to fau_register;
grant select on schema_contract to fau_register;

grant select on
  municipalities, municipality_names, municipality_numbers, municipality_slug_history,
  schools, school_slug_history, registered_faus, school_fau_links
  to fau_app;
grant insert on schools, school_submissions, register_lookups to fau_app;

alter table schools enable row level security;
create policy schools_app_read on schools for select to fau_app using (true);
-- fau_app may only ever queue a fresh, unreviewed submission: every field the
-- register itself later curates (a slug, an orgnr, the curated-name flag, closure
-- and its successor, verification and scope) must still be at its untouched
-- default (D9; tightened 24 September 2026 after review).
create policy schools_app_submit on schools for insert to fau_app
  with check (origin = 'submitted' and verification = 'pending' and slug is null
              and orgnr is null and not display_name_curated
              and status = 'active' and closed_on is null and closure_reason is null
              and successor_id is null and verified_at is null and scope_override is null
              and in_scope and last_seen_in_source_at is null and source_changed_at is null
              and register_name is null);
create policy schools_register_read on schools for select to fau_register using (true);
create policy schools_register_insert on schools for insert to fau_register with check (true);
create policy schools_register_update on schools for update to fau_register using (true) with check (true);
-- Guards against a mistaken delete of a live row, not against the role itself: this
-- is the only delete grant fau_register has, but nothing stops it from first
-- updating a row's verification to 'held' and then deleting it through here.
create policy schools_register_delete_held on schools for delete to fau_register using (verification = 'held');

-- fau_register curates every submission and lookup outcome; fau_app only ever
-- queues a fresh one and never reads or edits the register's own bookkeeping
-- back (D9 -- the runtime role must not assert register outcomes; tightened 24
-- September 2026 after review).
alter table school_submissions enable row level security;
create policy school_submissions_register_read on school_submissions for select to fau_register using (true);
create policy school_submissions_register_insert on school_submissions for insert to fau_register with check (true);
create policy school_submissions_register_update on school_submissions for update to fau_register using (true) with check (true);
create policy school_submissions_app_insert on school_submissions for insert to fau_app
  with check (review_state = 'pending' and reviewed_at is null
              and retrieval_status in ('fetched', 'not_allowlisted', 'failed', 'quarantined'));

alter table register_lookups enable row level security;
create policy register_lookups_register_read on register_lookups for select to fau_register using (true);
create policy register_lookups_register_insert on register_lookups for insert to fau_register with check (true);
create policy register_lookups_register_update on register_lookups for update to fau_register using (true) with check (true);
create policy register_lookups_app_insert on register_lookups for insert to fau_app
  with check (processed_at is null and outcome is null and attempts = 0);

insert into schema_contract (version) values (4);
