#!/bin/sh
# Local development only. Production credentials are provisioned out of band, per
# ADR-001's role split (docs/repo-container-contract.md) -- see #3424.
#
# `roles.sql` (this directory) creates the `fau_migrate` and `fau_app` roles without
# ever granting them a way to log in, on purpose: it is committed, and a committed
# file must never carry a password, even a throwaway local one. This script is what
# grants that login, from the environment `compose.yaml` passes to the `db`
# container -- never from a value written to disk here.
#
# PostgreSQL's initdb entrypoint runs every `/docker-entrypoint-initdb.d/*.sh` and
# `*.sql` file in filename order, so this is named `zz-compose-init.sh` specifically
# to sort after `roles.sql`: the roles must exist before this can alter them.
#
# The heredoc is quoted (`<<'SQL'`) so the shell does not touch its contents at all;
# the passwords instead reach `psql` as `-v` variables and are substituted with the
# `:'name'` form, which has `psql` itself quote the value as a SQL string literal
# (escaping any embedded `'` or `\`). Splicing `$FAU_APP_PASSWORD` directly into the
# SQL text -- the shell's own `'...'` quoting -- breaks the moment a chosen password
# contains a single quote; this does not.
set -eu
psql -v ON_ERROR_STOP=1 \
     -v migrate_pw="${FAU_MIGRATE_PASSWORD:-fau_migrate}" \
     -v app_pw="${FAU_APP_PASSWORD:-fau_app}" \
     -v register_pw="${FAU_REGISTER_PASSWORD:-fau_register}" \
     -U "$POSTGRES_USER" -d "$POSTGRES_DB" <<'SQL'
alter role fau_migrate  login password :'migrate_pw';
alter role fau_app      login password :'app_pw';
alter role fau_register login password :'register_pw';
SQL
