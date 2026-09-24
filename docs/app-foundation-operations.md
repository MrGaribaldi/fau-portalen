# Operating the application foundation locally

Companion to docs/app-foundation-design.md §14-15 (ADR-001,
docs/repo-container-contract.md). Covers the local Compose stack `compose.yaml`
builds -- `db`, `migrate`, `app`, `mail` -- and the tests that exercise it. Nothing
here is production operations: production role provisioning, deployment ordering and
probe manifests are #3424's work, referenced below only as the precondition this
stack's design assumes.

**If `db` is already running from a volume created before this stack had its init
scripts**, fix that first -- see "An existing volume created before the init
scripts" below. Otherwise `migrate` fails authentication and `app` never starts.

## Starting the stack

A fresh checkout with no `.env` yet:

```
cp .env.example .env    # edit host paths and, if needed, host ports
docker compose up --build -d --wait
```

If `.env` already exists -- as it does on the agent box and on Erik's own host,
holding `FAU_HOST_PATH` and `TTYD_PASSWORD` -- **do not overwrite it** with `cp`.
Append the application section from `.env.example` (`POSTGRES_PASSWORD` through
`FAU_MAIL_PORT`) to the existing file instead, or skip that step entirely: every one
of those variables has a matching default in `compose.yaml` (`${VAR:-default}`), so
appending them to `.env` is only useful for overriding a default, never required to
start the stack.

```
docker compose up --build -d --wait
```

`--wait` blocks until `db` reports healthy, `migrate` has exited 0, and `app`'s own
healthcheck (a `GET /health/ready` run *inside* the container) reports healthy in
turn -- Compose will not consider the stack up on ordering alone. Reach the app at
`http://localhost:${FAU_APP_PORT:-8000}`, and the mail test adapter's web UI at
`http://localhost:${FAU_MAIL_PORT:-8025}`.

`compose.yaml`'s bind mount for `backend/db` resolves on the **host** Docker daemon,
not inside whatever container runs the `docker compose` command -- from this agent
container, `docker` talks to the host daemon over the mounted socket, so a plain `.`
would resolve against the host's own current directory, not this container's
`/workspace`. `FAU_HOST_PATH` in `.env` is the repository's absolute path as the host
sees it; a plain checkout (a real host, no agent-box container in between) can leave
it unset and rely on the `.` fallback, since there the compose command's working
directory already resolves correctly on its own daemon.

## An existing volume created before the init scripts

PostgreSQL only runs `/docker-entrypoint-initdb.d/*` the first time it initialises a
cluster inside an **empty** data directory. A volume that already holds a cluster --
created before `compose.yaml` mounted `backend/db` there at all, or before
`zz-compose-init.sh` existed alongside `roles.sql` -- skips every init script
forever, no matter what is later added to that directory. Concretely: `db` is
recreated on the next `docker compose up` (a new container, same volume attached),
but its data directory is not empty, so `roles.sql` and `zz-compose-init.sh` never
run against it. `fau_migrate` and `fau_app` end up missing, or exist without a login
password; `migrate` fails authentication; `app` never starts, since it waits on
`migrate`'s successful completion.

The dev stack's own `fau-app-db-1` was in exactly this state when this stack was
introduced on 23 September 2026, because its volume predates both init scripts. If
`migrate` fails authentication against it, that is the likely cause. Two ways out:

**(a) Keep the data.** Apply `roles.sql` by hand against the running database, then
grant login exactly as `zz-compose-init.sh` would have on a fresh volume:

```
docker compose exec -T db psql -U postgres -d fau -v ON_ERROR_STOP=1 < backend/db/roles.sql
docker compose exec -T db psql -U postgres -d fau -v ON_ERROR_STOP=1 -c \
  "alter role fau_migrate login password 'fau_migrate'"
docker compose exec -T db psql -U postgres -d fau -v ON_ERROR_STOP=1 -c \
  "alter role fau_app login password 'fau_app'"
docker compose exec -T db psql -U postgres -d fau -v ON_ERROR_STOP=1 -c \
  "alter role fau_register login password 'fau_register'"
```

Replace the three literal passwords with whatever `.env` actually sets for
`FAU_MIGRATE_PASSWORD` / `FAU_APP_PASSWORD` / `FAU_REGISTER_PASSWORD` --
`fau_migrate`/`fau_app`/`fau_register` above are only `.env.example`'s defaults. Do
this once; every later `docker compose up` finds the roles already correctly
configured, and `zz-compose-init.sh` is simply never invoked again on this volume (it
is still skipped, but it no longer needs to run).

The same applies to `fau_register` (#3441, migration 0004, `db/roles.sql`): a volume
already initialised before this role existed never re-runs `zz-compose-init.sh`, so
`fau_register` gets no login password on it until either the commands above are run
by hand or the volume is recreated (option (b) below) so the init scripts run fresh.

**(b) Recreate the volume.** `docker compose down -v` removes the named volume along
with the containers -- **this destroys the dev database and everything in it.** Only
do this if the data in that volume is disposable, which for a local dev stack is
usually true, but confirm before running it against anything that is not. The next
`docker compose up --build -d --wait` then initialises a fresh, empty data directory,
and both init scripts run as designed.

## Upgrading an existing database volume

Migrations are embedded into the `fau` binary at **compile time**
(`sqlx::migrate!`, forced to rebuild on any change under `backend/migrations/` by
`persistence/build.rs`), and `migrate`/`app` both reference the image by the mutable
tag `fau/app:dev` -- not a content-addressed digest that would force Compose to
notice new source on its own. So pulling code that adds a migration and then running
`docker compose up -d --wait` **without** `--build` reuses whatever `fau/app:dev`
already points at: `migrate` runs, finds nothing new to apply (its own embedded
migration set has not changed), and `app` starts against a schema now behind the
checkout, with no warning. The upgrade command must always rebuild first, so it is
the same command as a first start:

```
docker compose up --build -d --wait
```

If `migrate` needs to be run on its own -- to watch its output directly, or to retry
after a failure without restarting the rest of the stack -- rebuild first, then run
it directly:

```
docker compose build && docker compose run --rm migrate
```

Migrations are immutable once applied (design §5): none of this ever rewrites
history, it only ensures `migrate` is running a binary that actually contains
whatever is pending.

## Role provisioning: the production precondition

`backend/db/roles.sql` creates the `fau_migrate` and `fau_app` roles without ever
granting them a way to log in -- it is committed, so it must never carry a password,
even a throwaway local one. `backend/db/zz-compose-init.sh` is what grants that login
locally, from the environment `compose.yaml` passes to the `db` container; it is
named to sort after `roles.sql` because PostgreSQL's initdb entrypoint runs
`/docker-entrypoint-initdb.d/*` in filename order and the roles must exist before
this can alter them. **Neither file is a production provisioning mechanism.**
Production credentials are granted out of band, by #3424 -- retrofitting the
migration/runtime privilege split after data exists is considerably harder than
starting with one, which is why both roles are created here from the first
migration rather than added later.

## The two lock variables, and when to raise them

`fau migrate` serialises itself against other migrators (design §5), so two
concurrent migration jobs never apply the same file twice: one applies, the other
waits and then finds nothing pending. It needs two different bounds, which is why
there are two variables (`backend/crates/persistence/src/migrate.rs` explains the
mechanism in its module docs):

- `MIGRATION_LOCK_WAIT_MS` (default `30000`, 30s) -- how long `migrate` queues
  behind another migrator that already holds the run-level lock before failing with
  "migration lock unavailable". Queueing is normal when several runners start
  together, as in a rolling deploy. Raise this when migrations are slow and several
  runners may start at the same time, so the later ones outwait the first rather
  than failing.
- `MIGRATION_LOCK_TIMEOUT_MS` (default `10000`, 10s) -- PostgreSQL's `lock_timeout`
  on the migration connection. It bounds how long each DDL statement may wait for a
  table lock held by live traffic, so a migration cannot queue behind production
  queries (and make every later query queue behind it) for long. It also backstops
  sqlx's own advisory lock, which the run-level lock should already have made
  uncontended. Raise it only knowingly: a higher value lets a migration stall live
  traffic for longer.

`compose.yaml` does not pass either variable to the `migrate` service, so it always
runs with the defaults. Raising one means adding it to that service's
`environment:` block (optionally reading it from `.env` the same way the other
variables do) -- editing `.env` or `.env.example` alone changes nothing `migrate`
reads. Do not raise either as a blanket workaround for an intermittent failure; that
usually means something else is wrong, such as a stuck connection or a long-running
transaction holding a table lock.

## `compose.yaml` and `docker-compose.yml` are different stacks

`/workspace` carries two Compose files on purpose. `docker-compose.yml` is the
agent-box stack (project `fau-mvp`): the `agent` service this session runs in, plus
data volumes. `compose.yaml` is the FAU application stack (project `fau-app`): `db`,
`migrate`, `app`, `mail`. Compose prefers `compose.yaml` when both are present in a
directory, so a bare `docker compose <command>` run from `/workspace` always targets
the application stack, and the agent-box stack always requires an explicit `-f
/workspace/docker-compose.yml` -- which is already the documented practice for it.
This is deliberate, not an oversight to reconcile: it means a stray `docker compose
down` typed without thinking cannot take down the container the work is happening
in.

The corollary matters for testing: any test or ad-hoc command that drives
`compose.yaml` from inside this agent container is one `docker compose down` away
from taking down the very session issuing it, if it is ever pointed at the wrong
project. See "Running the test suites" below for how the acceptance test avoids
that -- it never targets `fau-app`.

## Reaching the dev stack from this agent container

This agent container's own project is `fau-mvp` (`docker-compose.yml`), separate
from the FAU application stack's `fau-app` (`compose.yaml`). Integration tests
running in this container cannot reach `db` through its published host port: this
container's `docker` CLI talks to the *host* Docker daemon over a mounted socket,
but the container itself has no network route to that host's `127.0.0.1` --
publishing a port on the host daemon does not make it reachable from here. Reaching
`db` directly instead means attaching this container to the application stack's own
network:

```
docker network connect fau-app_default <this agent container>
TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres
```

That attachment is lost whenever the agent container is recreated (not merely
restarted), and must be redone; reattaching it also reloads `ttyd`, so expect the
terminal session to blip. Treat `fau-app_default` as something this container must
stay attached to for as long as tests need to reach `db` this way -- detaching or
removing it severs exactly what this section sets up.

## Running the test suites

Every suite that touches PostgreSQL needs `TEST_DATABASE_URL`, a superuser URL for
the cluster the harness creates its per-test databases in (one database per test,
cloned from a migrated template -- design §15). This is the canonical set, and what
CI should run, in this order:

```
cd backend
export TEST_DATABASE_URL=postgres://postgres:postgres@db:5432/postgres

# Unit and integration tests.
cargo test --workspace

# The shutdown and panic-handling tests exist only with the `test-routes` feature,
# which adds routes that give tests a handle on hard-to-trigger behaviour
# (`/test/slow`, `/test/slow-write`, `/test/panic`). Never compiled into the image.
cargo test -p fau-app --features test-routes

# Ignored: build and inspect the real image, then drive the real compose.yaml.
cargo test -p fau-app --test image -- --ignored --test-threads=1
cargo test -p fau-app --test acceptance -- --ignored --test-threads=1
```

**The harness resets role passwords on the target cluster.** Roles are cluster-wide,
and the harness runs `alter role fau_app login password 'fau_app'`,
`alter role fau_migrate login password 'fau_migrate'` and
`alter role fau_register login password 'fau_register'` (each role's password is its
own name) on whatever cluster `TEST_DATABASE_URL` points at. Pointing the suite at
the dev Compose `db`, as above, therefore overwrites any custom `FAU_APP_PASSWORD`,
`FAU_MIGRATE_PASSWORD` or `FAU_REGISTER_PASSWORD` set in `.env` for that cluster: the
dev stack's `migrate` and `app` then fail authentication until the roles are altered
back (see "Keep the data" above for the commands). With the defaults the three agree
and nothing changes. Never point `TEST_DATABASE_URL` at a cluster whose role
passwords matter.

The image-contract test (`backend/crates/app/tests/image.rs`) builds the real
`fau/app:dev` image with the real `Dockerfile` and inspects it -- non-root,
read-only root filesystem, no build toolchain or secrets reaching the runtime stage.

The acceptance test (`backend/crates/app/tests/acceptance.rs`) drives the real
`compose.yaml` end to end: a clean start, a row written directly through the
database, a restart (volume kept), and proof the row survived plus that `migrate`
exits 0. It runs under its own Compose project, `fau-acceptance`, on its own fixed
host ports -- **never** the developer's own `fau-app` project -- so it can run
safely alongside a dev stack that is already up.

Both tests rebuild `fau/app:dev` from the working tree -- the same shared tag the dev
stack's own `migrate`/`app` services use. Running them therefore replaces whatever
`fau/app:dev` currently is with a build of whatever is on disk right now; that is
normally what you want (it proves the working tree builds and runs), but it means
the dev stack's `migrate`/`app`, if restarted afterwards without their own rebuild,
will pick up that same image too.

Both ignored suites mutate real Docker state (images, containers, volumes,
networks) and are slow, which is why they are `#[ignore]`d rather than part of the
default `cargo test` run; `--test-threads=1` matters for the acceptance test
specifically, since its test functions each drive the same `fau-acceptance` project
and would otherwise race each other's `up`/`down`.

## Port variables

Every published port is parameterised, defaulting to what a plain checkout expects:

| Variable | Default | Publishes |
| --- | --- | --- |
| `FAU_DB_PORT` | `5433` | PostgreSQL, for a local client or `TEST_DATABASE_URL` |
| `FAU_APP_PORT` | `8000` | The application HTTP surface |
| `FAU_MAIL_PORT` | `8025` | The mail test adapter's web UI |

Raise any of these in `.env` only if the default collides with something else
already running on the host -- the acceptance test above sets its own fixed,
non-default values (`15433`/`18000`/`18025`) precisely so it never needs to.
