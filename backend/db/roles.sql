-- Database roles for FAU. Cluster-level objects, so not a migration.
-- Applied by the Compose init script and by the integration-test harness;
-- #3424 provisions the same three names in production. Names are a fixed project
-- contract: migration 0002 grants to `fau_app` by name.
--
-- No passwords here. roles.sql is committed; credentials are granted out of band --
-- the Compose init script for a real deployment, `apply_roles` in the test harness.

-- Role creation is wrapped in exception handling rather than an `if not exists`
-- check: "check, then create" is not atomic, so two processes racing to build a
-- test database at the same time (this file is applied by every fresh and every
-- migrated-template database) can both pass the check before either creates the
-- role. `create role` normally raises `duplicate_object` (SQLSTATE 42710) when the
-- role already exists, but the concurrent *loser* of the race more often gets
-- `unique_violation` (23505) instead -- the two backends both pass the check, then
-- collide on `pg_authid`'s name index, and whichever commits second sees a unique-
-- index conflict rather than the "already exists" check. Catching both makes the
-- statement itself safe under the race regardless of which one comes back.
do $$
begin
  begin
    create role fau_migrate nologin;
  exception when duplicate_object or unique_violation then
    null;
  end;
  begin
    create role fau_app nologin;
  exception when duplicate_object or unique_violation then
    null;
  end;
  begin
    create role fau_register nologin;
  exception when duplicate_object or unique_violation then
    null;
  end;
end
$$;

-- `current_database()` cannot appear directly in `grant ... on database` / `revoke
-- ... on database`, so the statements are generated instead.
do $$
begin
  execute format('grant create, connect on database %I to fau_migrate', current_database());
  execute format('grant connect on database %I to fau_app', current_database());
  execute format('grant connect on database %I to fau_register', current_database());
  -- Withhold TEMP too: the runtime role has no business creating temporary tables,
  -- and PostgreSQL grants CONNECT and TEMPORARY together by default.
  execute format('revoke temporary on database %I from public', current_database());
end
$$;

-- The migration role owns DDL: it needs both usage of, and create rights in, the
-- public schema -- PostgreSQL 15 stopped granting either to PUBLIC by default, so
-- without this a migration's `create table` fails with permission denied.
grant usage, create on schema public to fau_migrate;

-- The runtime role gets connect and usage only; table privileges are granted per
-- table by the migration that creates it, so a new table is inaccessible to the
-- runtime until someone decides what it may do.
grant usage on schema public to fau_app;

-- The register role (#3441, D9) alone writes the school register, and runs the
-- weekly sync and the submission lookups. Table privileges come from migration 0004.
grant usage on schema public to fau_register;
revoke create on schema public from fau_register;

-- Explicitly withhold the default. Without this, PUBLIC can create objects in the
-- public schema on PostgreSQL versions before 15 and, more importantly, the intent
-- is recorded where a reviewer will read it.
revoke create on schema public from public;
revoke create on schema public from fau_app;
