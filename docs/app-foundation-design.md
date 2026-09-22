# Application foundation: design for #3416

Status: proposed, 22 September 2026. Design for the first application code in FAU — the Rust
backend skeleton, migrations, container and local stack. Approved in conversation with Erik the
same day; this document is the handoff to an implementation plan.

Implements the contract in ADR-001 (docs/repo-container-contract.md). Depends on the approved
tenant, role and history model on #3412, ADR-003 (docs/identity-and-encryption.md) for identity and
encryption, and the localisation decisions on #3439.

## 1. Scope

**In:** a Rust binary with `serve`, `migrate` and `--version`; the schema-contract mechanism and the
first two migrations; two database roles; health probes; structured logging that the existing
telemetry stack understands; the error contract; graceful shutdown; a production image; the local
Compose stack; and integration tests against real PostgreSQL.

**Out, deliberately:** the frontend build pipeline and any UI beyond a static placeholder — those
stay with #3422 and #3423, which inherit a working backend rather than a half-built frontend.
Authentication against Hanko (#3417). Documents, folders, tags and the collaboration authority
(#3419, #3490). Row-level security (#3418, see section 7). Mail delivery beyond a local test
adapter (#3410).

The acceptance question for this increment is narrow: **can someone clone the repository, copy
`.env.example` to `.env`, run `docker compose up --build`, and reach a healthy application on
`http://localhost:8000` — repeatedly, including against an existing database volume?**

## 2. Repository layout

```text
backend/
  Cargo.toml, Cargo.lock, rust-toolchain.toml
  crates/domain/        entities, role periods, capability rules. No HTTP, no SQL.
  crates/persistence/   sqlx; owns transaction boundaries
  crates/app/           axum wiring, configuration, the `fau` binary
  migrations/           0001_schema_contract.sql, 0002_identity_and_tenancy.sql
  tests/                integration against real PostgreSQL
Dockerfile
compose.yaml
.env.example
```

`domain` declares neither `axum` nor `sqlx` in its manifest, so ADR-001's rule that the domain does
not depend on HTTP or UI is enforced by the dependency graph rather than by memory. `persistence`
owns the transaction boundaries #3412 requires around membership, role assignment and audit.

**On `compose.yaml` at the root.** `/workspace` already carries `docker-compose.yml` for the
agent-box stack. Compose prefers `compose.yaml` when both exist, so a bare `docker compose` command
targets the application and the agent stack always requires `-f /workspace/docker-compose.yml` —
which is already the documented practice. This is deliberate: a stray `docker compose down` cannot
take down the container the work is happening in.

## 3. Process contract

| Invocation | Behaviour |
| --- | --- |
| `fau serve` | Binds `HTTP_BIND`, default `0.0.0.0:8000`. Never performs DDL. |
| `fau migrate` | Applies pending migrations, exits 0 on success and non-zero on failure. |
| `fau --version` | Prints the build revision. Reads no configuration and touches no secret. |

One binary, one image, one digest across test, migration and deploy — which is what makes ADR-001's
release barrier meaningful rather than aspirational.

## 4. Configuration

From ADR-001's table. This increment requires `APP_ENV`, `HTTP_BIND`, `PUBLIC_BASE_URL`,
`DATABASE_URL`, `DB_POOL_MAX_CONNECTIONS` and `LOG_LEVEL`; `migrate` requires only
`MIGRATION_DATABASE_URL`. Telemetry and mail variables are accepted and unused until their adapters
arrive.

Invalid or missing required configuration fails at startup with the **variable name, never its
value**, and a non-zero exit. A database that is merely unreachable is not a configuration error:
readiness goes false with bounded retry and the process stays up, because a restart loop during a
brief database outage turns a recoverable incident into an outage of our own.

## 5. Migrations and the schema contract

Two bookkeeping concerns, kept separate because they answer different questions.

`_sqlx_migrations` is sqlx's own table and answers *which files have run*. It records a checksum per
migration and treats a changed checksum on an applied migration as an error, and the migrator holds
a PostgreSQL advisory lock for the whole run, so two concurrent jobs cannot apply the same migration
in parallel. That satisfies ADR-001's requirements directly.

`schema_contract` is ours and answers *whether this binary can serve this database*. The
application declares a minimum contract version and **tolerates a higher one**, which is what lets
an additive migration roll out before the new pods do, per ADR-001's expand/contract rule.

**`0001_schema_contract.sql`** creates that table and nothing else, proving the runner end to end
before any domain structure depends on it:

```sql
create table schema_contract (
  version    integer     primary key,
  applied_at timestamptz not null default now()
);
insert into schema_contract (version) values (1);
```

The application reads `max(version)` and refuses to serve when it is below its own minimum.

**`0002_identity_and_tenancy.sql`** creates the spine, reconciling #3412 with ADR-003 and #3439:

| Table | Key columns |
| --- | --- |
| `accounts` | `id` uuid pk, `email` unique, `verified_at`, `disabled_at`, `locale` nullable (#3439), `retention_months` not null default 3 (ADR-003 6a) |
| `identity_mappings` | `(issuer, subject)` pk → `account_id`. **Several rows per account by design** |
| `tenants` | `id` uuid pk, `name`, `status` constrained to pending/active/closed, `school_id` nullable, `default_locale` not null default `nb-NO` |
| `memberships` | pk `(tenant_id, id)`, `account_id`, `revoked_at`, unique `(tenant_id, account_id)` |
| `roles` | pk `(tenant_id, id)`, `name`, `capability_class` constrained to member/admin, optional `unit_id`/`cohort_id` |
| `role_assignments` | pk `(tenant_id, id)`, `membership_id`, `role_id`, `starts_on`, `ends_on_exclusive` not null, `revoked_at`, `granted_by` |

Three properties are load-bearing and must not be softened later:

1. **Every reference between tenant data uses the composite key `(tenant_id, id)`**, so a
   cross-tenant reference is rejected by the database rather than by application code that might
   forget. This is #3412's central integrity rule.
2. **Several identity mappings per account.** ADR-003 decision 3 names this the single rule most
   expensive to add later: it is what allows two identity providers to run concurrently during a
   migration instead of forcing a cutover.
3. **Role periods are half-open `[starts_on, ends_on_exclusive)`** with the end mandatory and a
   `check (starts_on < ends_on_exclusive)`. Dates are local calendar dates in `Europe/Oslo`;
   event timestamps are `timestamptz` in UTC. The server's clock decides access, never the
   browser's.

**No key material of any kind appears in any application table.** Document and tenant keys live only
in the key service (ADR-003 decision 5). A schema review that finds a wrapped key on a row is a
defect, and there is a test for it.

## 6. Database roles

ADR-001 gives `DATABASE_URL` and `MIGRATION_DATABASE_URL` as separate accounts. The migration role
owns DDL; the runtime role has neither DDL nor rights over finalised history and audit. Both are
created as part of this increment, because retrofitting a privilege split after data exists is
considerably harder than starting with one.

## 7. Row-level security, deferred with reasons

#3412 proposes RLS as a second isolation layer. It is not in this increment. It needs the runtime
role plus transaction-scoped tenant context, and enabling policies while no request path exercises
them risks everything silently running as the migration role and masking the gap — a defence that
looks present and is not. It belongs to #3418, where endpoints exist to prove isolation against.
The composite-key rule in section 5 is the isolation guarantee that ships now.

## 8. HTTP surface

This increment serves `/health/live`, `/health/ready`, and a static placeholder at `/`. The
`/api/v1/` prefix and the `/app/` fallback rule are established but carry no routes yet. Unknown API
routes return 404 and never fall back to HTML.

## 9. Health and readiness

`/health/live` touches the process only — no database, mail, RabbitMQ, S3 or OTLP, per ADR-001, so
that a dependency outage cannot cause a restart loop.

`/health/ready` requires local initialisation complete, a database check with a one-second timeout,
and a schema contract at or above the binary's minimum. It returns 503 otherwise, and caches its
result briefly so that probes do not themselves become database load.

## 10. Logging and telemetry

**Verified against the deployed stack on 22 September 2026**, not assumed.

infra-tools runs Grafana Alloy as a DaemonSet with `loki.source.kubernetes`, which reads every pod's
stdout and ships it to Loki. Its pipeline runs `stage.json` over each line, extracts a top-level
`level` field, normalises it to DEBUG/INFO/WARNING/ERROR/CRITICAL and promotes it to a Loki label.

Consequences, each of which changes what we build:

- **JSON to stdout is the log transport.** `tracing-subscriber`'s JSON formatter emits `level` with
  conventional names, so the existing Grafana dashboards filter our logs with no configuration on
  either side.
- **Logs must not also be exported over OTLP.** The Alloy DaemonSet has an `otelcol.receiver.otlp`,
  so exporting logs there as well would collect each line twice — exactly what ADR-001 warns
  against when it says to choose one transport. OTLP stays reserved for traces to Tempo.
- **Do not depend on a `service_name` label.** Alloy drops it as high-cardinality. Identification
  comes from `app.kubernetes.io/name`, which Alloy relabels from the pod, so the Deployment must
  carry that label. That is #3424's work, recorded here because "our logs do not appear in Grafana"
  is an unpleasant thing to diagnose later.
- **Collection is not opt-in.** No annotation is required; every pod is scraped.
- **Alloy drops lines older than 48 hours at ingest**, independently of Loki's retention. This is a
  concrete number behind ADR-003's rule that business audit belongs in PostgreSQL and not in
  operational logs.

Each log line carries timestamp, level, service version, request ID, **route template**, status and
duration. The route template comes from axum's `MatchedPath` rather than the URI, which is what
keeps identifiers out of the logs.

Never logged: query strings, email addresses, cookies, `Authorization`, magic-link tokens, SQL
parameters, document content.

## 11. Error contract

API errors are JSON carrying a stable error code, bounded parameters and the request ID — **never
display text**. #3439 made this load-bearing for localisation: the client renders the sentence, so
an endpoint that returns Norwegian prose silently breaks translation. The same rule applies to
audit, which stores action codes rather than rendered sentences.

## 12. Shutdown

SIGTERM sets readiness to false, stops accepting new work, and allows in-flight requests and
transactions up to 25 seconds to finish before the process exits. An aborted transaction is never
acknowledged to the client.

## 13. Container image

Multi-stage, base images pinned by digest. The runtime stage carries the binary, CA certificates and
`migrations/`, and nothing else. It runs as a non-root user with a read-only root filesystem and a
tmpfs `/tmp`. No documents or durable state are written inside the container.

## 14. Local Compose stack

Services `db`, `migrate`, `app` and a local mail test adapter with no external delivery. `db` must
be healthy before `migrate` runs, and `app` starts only after `migrate` completes successfully —
expressed with `service_healthy` and `service_completed_successfully`, because ordering alone does
not prove a ready database.

PostgreSQL's major version matches the verified production target; the exact patch version is pinned
at implementation. The database volume is local and persistent, and the upgrade path for an existing
volume is documented: a previously completed one-shot container does not re-run on its own.

## 15. Test plan

Integration tests run against real PostgreSQL. Each test gets its own database created from a
template and dropped afterwards, rather than a wrapping transaction — transaction isolation breaks
as soon as the code under test manages its own transactions, which persistence does.

Tests are written before the code they exercise.

ADR-001 names three pieces of evidence for this card, and they are the backbone of the suite:

| Scenario | Expected |
| --- | --- |
| Clean checkout, then restart against an existing database volume | Content preserved; pending migrations applied |
| Image contract | `serve` answers on 8000; `migrate` exit status correct; non-root; read-only root filesystem; routing honoured |
| Database down, then back | `/health/live` stays 200; `/health/ready` 503 then 200; no restart loop; no acknowledged write lost |

Added because the card requires them:

| Scenario | Expected |
| --- | --- |
| Known sentinel secrets in configuration, routes exercised | No sentinel value appears anywhere in captured log output |
| Migration checksum altered after being applied | `migrate` fails and exits non-zero |
| Two `migrate` processes started concurrently | One applies; the other waits and then finds nothing to do; never both |
| Migration lock held beyond the bound | Fails rather than hanging — `lock_timeout` is set on the migration connection |
| Cross-tenant reference inserted directly in SQL | Rejected by the composite foreign key |
| Role assignment with `ends_on_exclusive` ≤ `starts_on` | Rejected by the check constraint |
| Binary started against a database whose schema contract is below its minimum | Refuses to serve, with a clear message |
| Binary started against a database with a *higher* contract | Serves — additive migrations must not lock out running pods |
| Any application table containing key material | Must not exist; caught by schema review test |

## 16. Open for implementation

Exact crate versions and the pinned base image digests. The PostgreSQL patch version. Whether the
static placeholder is served from the binary or from a file in the image. The precise
`lock_timeout` value. The initial `schema_contract` minimum the binary declares.

## 17. Sources

docs/repo-container-contract.md (ADR-001); the approved tenant, role and history design on #3412;
docs/identity-and-encryption.md (ADR-003); docs/localisation-design.md (#3439);
docs/structured-document-architecture.md (#3490) for what this increment deliberately leaves alone;
and the infra-tools telemetry-agent Alloy configuration, read directly on 22 September 2026.
